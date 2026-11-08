using System;
using System.Collections.Generic;
using System.Globalization;

namespace CodexUsageTray
{
    // Live observations use monotonic time and are never restored across restarts.
    internal sealed class BatteryRateTracker
    {
        private const double MaxGapSeconds = 7200;
        private const double MinimumPeriodSeconds = 1800;
        private const int MinimumChangePercent = 3;
        private const double MaxSegmentSeconds = 7 * 24 * 3600;
        private const double MaxRatePerHour = 200;
        private const int MaxHistoryDevices = 512;
        private const int MaxHistorySamplesPerDirection = 20;
        private const int MaxDeviceIdLength = 2048;
        private readonly Dictionary<string, Reading> readings =
            new Dictionary<string, Reading>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, LearnedBatteryRate> history =
            new Dictionary<string, LearnedBatteryRate>(StringComparer.OrdinalIgnoreCase);

        private sealed class Reading
        {
            public int StartPercent;
            public int LastPercent;
            public double StartTime;
            public double LastTime;
            public int State;
            public int Direction;
            public int Samples;
        }

        public long HistoryRevision { get; private set; }

        public void ImportHistory(IEnumerable<LearnedBatteryRate> values)
        {
            readings.Clear();
            history.Clear();
            HistoryRevision = 0;
            if (values == null) return;
            foreach (LearnedBatteryRate value in values)
            {
                if (value == null || !HasStableId(value.DeviceId) || history.ContainsKey(value.DeviceId)) continue;
                LearnedBatteryRate copy = new LearnedBatteryRate { DeviceId = value.DeviceId };
                CopyValidSamples(value.ChargeSamples, copy.ChargeSamples);
                CopyValidSamples(value.DischargeSamples, copy.DischargeSamples);
                if (copy.ChargeSamples.Count == 0 && copy.DischargeSamples.Count == 0) continue;
                history.Add(copy.DeviceId, copy);
                if (history.Count >= MaxHistoryDevices) break;
            }
        }

        public List<LearnedBatteryRate> ExportHistory()
        {
            List<LearnedBatteryRate> result = new List<LearnedBatteryRate>();
            foreach (LearnedBatteryRate value in history.Values) result.Add(value.Clone());
            result.Sort(delegate(LearnedBatteryRate left, LearnedBatteryRate right)
            {
                return StringComparer.OrdinalIgnoreCase.Compare(left.DeviceId, right.DeviceId);
            });
            return result;
        }

        // Resume, disconnect, and gaps invalidate only the live segment, not its historical averages.
        public void Reset()
        {
            readings.Clear();
        }

        public void Update(IList<BatteryProbeResult> devices, double now)
        {
            if (devices == null || !IsFinite(now) || now < 0)
            {
                readings.Clear();
                if (devices != null)
                    foreach (BatteryProbeResult device in devices) if (device != null) device.BatteryEstimate = null;
                return;
            }
            foreach (BatteryProbeResult device in devices)
            {
                if (device == null) continue;
                device.BatteryEstimate = null;
                if (string.IsNullOrWhiteSpace(device.Id)) continue;
                string rateId = LogitechHidBatteryCollector.IsPhysicalId(device.PhysicalId) ? device.PhysicalId : device.Id;
                int state = GetState(device.Status);
                if (state == 2)
                {
                    readings.Remove(rateId);
                    continue;
                }
                if (!device.Percent.HasValue || device.Percent.Value < 0 || device.Percent.Value > 100 || device.IsCached)
                {
                    continue;
                }

                LearnedBatteryRate legacyHistory;
                if (!string.Equals(rateId, device.Id, StringComparison.OrdinalIgnoreCase) &&
                    !history.ContainsKey(rateId) && history.TryGetValue(device.Id, out legacyHistory))
                {
                    history.Remove(device.Id);
                    legacyHistory.DeviceId = rateId;
                    history.Add(rateId, legacyHistory);
                    HistoryRevision++;
                }

                int percent = device.Percent.Value;

                Reading reading;
                if (!readings.TryGetValue(rateId, out reading) || state != reading.State ||
                    now <= reading.LastTime || now - reading.LastTime > MaxGapSeconds ||
                    now - reading.StartTime > MaxSegmentSeconds)
                {
                    readings[rateId] = NewReading(percent, state, now);
                    device.BatteryEstimate = HistoricalEstimate(rateId, state, 0, percent);
                    continue;
                }

                int direction = Math.Sign(percent - reading.LastPercent);
                if (direction != 0 && ((state == 1 && direction < 0) || (state == -1 && direction > 0) ||
                    (direction > 0 && string.Equals(device.Status, "Not charging", StringComparison.OrdinalIgnoreCase))))
                {
                    readings[rateId] = NewReading(percent, state, now);
                    device.BatteryEstimate = null;
                    continue;
                }
                if (direction != 0 && reading.Direction != 0 && reading.Direction != direction)
                {
                    Reading replacement = NewReading(percent, state, now);
                    replacement.Direction = direction;
                    readings[rateId] = replacement;
                    device.BatteryEstimate = HistoricalEstimate(rateId, state, direction, percent);
                    continue;
                }
                if (direction != 0) reading.Direction = direction;
                reading.LastTime = now;
                reading.LastPercent = percent;
                reading.Samples++;

                int change = percent - reading.StartPercent;
                double elapsed = now - reading.StartTime;
                if (reading.Samples < 3 || elapsed < MinimumPeriodSeconds || Math.Abs(change) < MinimumChangePercent)
                {
                    device.BatteryEstimate = HistoricalEstimate(rateId, state, reading.Direction, percent);
                    continue;
                }

                int qualifiedDirection = Math.Sign(change);
                double perHour = Math.Abs(change) * 3600.0 / elapsed;
                LearnedBatteryRate existing;
                bool hadDirection = history.TryGetValue(rateId, out existing) &&
                    (qualifiedDirection > 0 ? existing.ChargeSamples.Count > 0 : existing.DischargeSamples.Count > 0);
                if (RecordSegment(rateId, qualifiedDirection, Math.Abs(change), elapsed))
                {
                    LearnedBatteryRate updated = history[rateId];
                    perHour = AverageRate(qualifiedDirection > 0 ? updated.ChargeSamples : updated.DischargeSamples);
                    Reading next = NewReading(percent, state, now);
                    next.Direction = qualifiedDirection;
                    readings[rateId] = next;
                }
                device.BatteryEstimate = FormatEstimate(hadDirection ? "Avg." : "Est.", qualifiedDirection, perHour, percent, true);
            }

            List<string> expired = new List<string>();
            foreach (KeyValuePair<string, Reading> entry in readings)
                if (now - entry.Value.LastTime > MaxGapSeconds) expired.Add(entry.Key);
            foreach (string id in expired) readings.Remove(id);
        }

        private string HistoricalEstimate(string id, int state, int direction, int percent)
        {
            LearnedBatteryRate value;
            if (!HasStableId(id) || !history.TryGetValue(id, out value)) return null;
            if (state == 1 && value.ChargeSamples.Count > 0)
                return FormatEstimate("Avg.", 1, AverageRate(value.ChargeSamples), percent, true);
            if (state == -1 && value.DischargeSamples.Count > 0)
                return FormatEstimate("Avg.", -1, AverageRate(value.DischargeSamples), percent, true);
            if (state != 0) return null;
            if (direction > 0 && value.ChargeSamples.Count > 0)
                return FormatEstimate("Avg. charge", 1, AverageRate(value.ChargeSamples), percent, false);
            if (direction <= 0 && value.DischargeSamples.Count > 0)
                return FormatEstimate("Avg. use", -1, AverageRate(value.DischargeSamples), percent, false);
            return null;
        }

        private static string FormatEstimate(string label, int direction, double perHour, int percent, bool allowTime)
        {
            string numberFormat = perHour < 0.01 ? "0.####" : "0.##";
            string estimate = label + " " + (direction > 0 ? "+" : "\u2212") +
                perHour.ToString(numberFormat, CultureInfo.CurrentCulture) + "%/h";
            if (allowTime && perHour > 0)
            {
                double seconds = (direction > 0 ? 100 - percent : percent) / perHour * 3600.0;
                if (seconds > 0 && seconds < int.MaxValue)
                    estimate += " \u00b7 ~" + TimeFormatter.FormatDuration((int)Math.Round(seconds, MidpointRounding.AwayFromZero)) +
                        (direction > 0 ? " to full" : " remaining");
            }
            return estimate;
        }

        private bool RecordSegment(string id, int direction, double change, double seconds)
        {
            if (!HasStableId(id)) return false;
            LearnedBatteryRate value;
            if (!history.TryGetValue(id, out value))
            {
                if (history.Count >= MaxHistoryDevices) return false;
                value = new LearnedBatteryRate { DeviceId = id };
                history.Add(id, value);
            }
            List<LearnedBatteryRateSample> samples = direction > 0 ? value.ChargeSamples : value.DischargeSamples;
            samples.Add(new LearnedBatteryRateSample { Percent = change, Seconds = seconds });
            if (samples.Count > MaxHistorySamplesPerDirection) samples.RemoveAt(0);
            HistoryRevision++;
            return true;
        }

        private static double AverageRate(List<LearnedBatteryRateSample> samples)
        {
            double percent = 0, seconds = 0;
            foreach (LearnedBatteryRateSample sample in samples) { percent += sample.Percent; seconds += sample.Seconds; }
            return percent * 3600.0 / seconds;
        }

        private static void CopyValidSamples(List<LearnedBatteryRateSample> source, List<LearnedBatteryRateSample> destination)
        {
            if (source == null) return;
            foreach (LearnedBatteryRateSample sample in source)
            {
                if (sample == null || !IsFinite(sample.Percent) || !IsFinite(sample.Seconds) ||
                    sample.Percent < MinimumChangePercent || sample.Percent > 100 ||
                    sample.Seconds < MinimumPeriodSeconds || sample.Seconds > MaxSegmentSeconds ||
                    sample.Percent * 3600.0 / sample.Seconds > MaxRatePerHour) continue;
                destination.Add(sample.Clone());
                if (destination.Count > MaxHistorySamplesPerDirection) destination.RemoveAt(0);
            }
        }

        private static bool HasStableId(string id)
        {
            if (string.IsNullOrWhiteSpace(id) || id.Length > MaxDeviceIdLength) return false;
            Guid generated;
            return !(id.StartsWith("device:", StringComparison.OrdinalIgnoreCase) &&
                Guid.TryParseExact(id.Substring(7), "N", out generated));
        }

        private static bool IsFinite(double value) { return !double.IsNaN(value) && !double.IsInfinity(value); }

        private static Reading NewReading(int percent, int state, double now)
        {
            return new Reading { StartPercent = percent, LastPercent = percent, StartTime = now,
                LastTime = now, State = state, Samples = 1 };
        }

        // 1 = charging, -1 = discharging, 2 = full or error, 0 = not known.
        private static int GetState(string status)
        {
            if (string.Equals(status, "Fully charged", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(status, "Battery error", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(status, "Temperature warning", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(status, "Charging error", StringComparison.OrdinalIgnoreCase)) return 2;
            if (string.Equals(status, "Nearly charged", StringComparison.OrdinalIgnoreCase) ||
                (!string.IsNullOrEmpty(status) && status.StartsWith("Charging", StringComparison.OrdinalIgnoreCase))) return 1;
            return string.Equals(status, "Discharging", StringComparison.OrdinalIgnoreCase) ? -1 : 0;
        }
    }
}
