using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;

namespace CodexUsageTray
{
    internal static class BatteryMonitorClient
    {
        private static readonly object CacheLock = new object();
        private static readonly object RateLock = new object();
        private static readonly BatteryRateTracker RateTracker = new BatteryRateTracker();
        private static readonly Dictionary<string, BatteryProbeResult> CacheById =
            new Dictionary<string, BatteryProbeResult>(StringComparer.OrdinalIgnoreCase);

        public static UsageSnapshot FetchUsage()
        {
            DateTime attemptedAt = DateTime.Now;
            List<BatteryProbeResult> collected = new List<BatteryProbeResult>();
            List<string> errors = new List<string>();

            TryCollect(
                "Windows",
                delegate { return WindowsBatteryPropertyCollector.Collect(); },
                collected,
                errors);
            TryCollect(
                "Bluetooth LE",
                delegate
                {
                    return BluetoothGattBatteryCollector.Collect(
                        delegate(Exception ex) { errors.Add("Bluetooth LE device: " + ex.Message); });
                },
                collected,
                errors);
            TryCollect(
                "Logitech",
                delegate
                {
                    return LogitechHidBatteryCollector.Collect(
                        delegate(Exception ex) { errors.Add("Logitech device: " + ex.Message); });
                },
                collected,
                errors);
            TryCollect(
                "HyperX",
                delegate
                {
                    return HyperxHidBatteryCollector.Collect(
                        delegate(Exception ex) { errors.Add("HyperX device: " + ex.Message); });
                },
                collected,
                errors);

            List<BatteryProbeResult> devices = MergeAndCache(collected, attemptedAt);
            lock (RateLock)
            {
                RateTracker.Update(devices, Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency);
            }
            devices.Sort(CompareDevices);

            UsageSnapshot snapshot = new UsageSnapshot();
            snapshot.LastAttempted = attemptedAt;
            snapshot.LastUpdated = attemptedAt;
            snapshot.Devices = new List<LimitWindow>();

            int knownCount = 0;
            int totalPercent = 0;
            int lowest = 101;
            for (int index = 0; index < devices.Count; index++)
            {
                BatteryProbeResult device = devices[index];
                LimitWindow window = CreateWindow(device);
                snapshot.Devices.Add(window);
                if (device.Percent.HasValue)
                {
                    knownCount++;
                    totalPercent += device.Percent.Value;
                    lowest = Math.Min(lowest, device.Percent.Value);
                }
            }

            snapshot.Weekly = snapshot.Devices.Count > 0
                ? snapshot.Devices[0]
                : null;
            snapshot.FiveHour = snapshot.Devices.Count > 1
                ? snapshot.Devices[1]
                : null;
            snapshot.LowestBatteryPercent = knownCount > 0
                ? (int?)lowest
                : null;
            snapshot.AverageBatteryPercent = knownCount > 0
                ? (int?)Math.Round(totalPercent / (double)knownCount)
                : null;
            snapshot.PlanType = devices.Count.ToString(CultureInfo.CurrentCulture) +
                (devices.Count == 1 ? " device" : " devices");

            if (devices.Count == 0)
            {
                snapshot.StatusMessage = errors.Count > 0
                    ? "No supported devices found; one or more checks failed."
                    : "No supported wireless battery devices found.";
            }
            else if (errors.Count > 0)
            {
                snapshot.StatusMessage = "Some device types could not be checked.";
            }

            return snapshot;
        }

        public static void ResetBatteryRates()
        {
            lock (RateLock)
            {
                RateTracker.Reset();
            }
        }

        internal static void InitializeBatteryRates(IEnumerable<LearnedBatteryRate> history)
        {
            lock (RateLock)
            {
                RateTracker.ImportHistory(history);
            }
        }

        internal static bool TryGetUpdatedBatteryRates(long savedRevision,
            out long revision, out List<LearnedBatteryRate> history)
        {
            lock (RateLock)
            {
                revision = RateTracker.HistoryRevision;
                if (revision <= savedRevision)
                {
                    history = null;
                    return false;
                }
                history = RateTracker.ExportHistory();
                return true;
            }
        }

        private delegate List<BatteryProbeResult> Collector();

        private static void TryCollect(
            string label,
            Collector collector,
            List<BatteryProbeResult> destination,
            List<string> errors)
        {
            try
            {
                List<BatteryProbeResult> results = collector();
                if (results != null)
                {
                    destination.AddRange(results);
                }
            }
            catch (Exception ex)
            {
                errors.Add(label + ": " + ex.Message);
            }
        }

        private static List<BatteryProbeResult> MergeAndCache(
            List<BatteryProbeResult> collected,
            DateTime attemptedAt)
        {
            List<BatteryProbeResult> hydrated = new List<BatteryProbeResult>();
            lock (CacheLock)
            {
                for (int index = 0; index < collected.Count; index++)
                {
                    BatteryProbeResult current = collected[index];
                    if (current == null)
                    {
                        continue;
                    }
                    Normalize(current, attemptedAt);
                    if (current.Percent.HasValue)
                    {
                        Cache(current);
                    }
                    else
                    {
                        BatteryProbeResult cached = FindCached(current);
                        if (cached != null)
                        {
                            current.Percent = cached.Percent;
                            current.ObservedAt = cached.ObservedAt;
                            current.IsCached = true;
                            current.Status = "Last reported " +
                                TimeFormatter.FormatClock(cached.ObservedAt) +
                                " · sleeping or unavailable";
                        }
                    }
                    hydrated.Add(current);
                }
            }

            Dictionary<string, BatteryProbeResult> merged =
                new Dictionary<string, BatteryProbeResult>(StringComparer.OrdinalIgnoreCase);
            HashSet<string> ambiguousNames = FindAmbiguousNames(hydrated);
            for (int index = 0; index < hydrated.Count; index++)
            {
                BatteryProbeResult candidate = hydrated[index];
                string key = ambiguousNames.Contains(NormalizeName(candidate.Name))
                    ? "id:" + candidate.Id
                    : GetMergeKey(candidate);
                BatteryProbeResult existing;
                if (!merged.TryGetValue(key, out existing) ||
                    DeviceScore(candidate) > DeviceScore(existing))
                {
                    merged[key] = candidate;
                }
            }

            return new List<BatteryProbeResult>(merged.Values);
        }

        private static void Normalize(BatteryProbeResult value, DateTime attemptedAt)
        {
            if (value == null)
            {
                return;
            }
            if (string.IsNullOrWhiteSpace(value.Id))
            {
                value.Id = "device:" + Guid.NewGuid().ToString("N");
            }
            if (string.IsNullOrWhiteSpace(value.Name))
            {
                value.Name = "Wireless device";
            }
            if (string.IsNullOrWhiteSpace(value.Source))
            {
                value.Source = "Windows";
            }
            if (string.IsNullOrWhiteSpace(value.Status))
            {
                value.Status = value.Percent.HasValue ? "Connected" : "Unavailable";
            }
            if (value.Percent.HasValue)
            {
                value.Percent = Math.Max(0, Math.Min(100, value.Percent.Value));
            }
            if (value.ObservedAt == DateTime.MinValue)
            {
                value.ObservedAt = attemptedAt;
            }
        }

        private static void Cache(BatteryProbeResult value)
        {
            BatteryProbeResult clone = value.Clone();
            CacheById[clone.Id] = clone;
        }

        private static BatteryProbeResult FindCached(BatteryProbeResult value)
        {
            BatteryProbeResult cached;
            if (CacheById.TryGetValue(value.Id, out cached))
            {
                return cached;
            }
            return null;
        }

        private static HashSet<string> FindAmbiguousNames(List<BatteryProbeResult> values)
        {
            Dictionary<string, string> idsByNameAndSource =
                new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            HashSet<string> ambiguous = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (BatteryProbeResult value in values)
            {
                string name = NormalizeName(value.Name);
                string key = name + "\0" + value.Source;
                string existingId;
                if (idsByNameAndSource.TryGetValue(key, out existingId))
                {
                    if (!string.Equals(existingId, value.Id, StringComparison.OrdinalIgnoreCase))
                    {
                        ambiguous.Add(name);
                    }
                }
                else
                {
                    idsByNameAndSource[key] = value.Id;
                }
            }
            return ambiguous;
        }

        private static string GetMergeKey(BatteryProbeResult value)
        {
            string name = NormalizeName(value.Name);
            if (!string.IsNullOrEmpty(name) &&
                name.IndexOf("wireless device", StringComparison.OrdinalIgnoreCase) < 0 &&
                name.IndexOf("usb receiver", StringComparison.OrdinalIgnoreCase) < 0)
            {
                return "name:" + name;
            }
            return "id:" + value.Id;
        }

        private static string NormalizeName(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
            {
                return "";
            }
            return value.Trim().ToLowerInvariant();
        }

        private static int DeviceScore(BatteryProbeResult value)
        {
            return (value.Percent.HasValue ? 1000 : 0) +
                (value.IsCached ? -100 : 0) +
                value.Priority;
        }

        private static int CompareDevices(BatteryProbeResult left, BatteryProbeResult right)
        {
            if (left.Percent.HasValue != right.Percent.HasValue)
            {
                return left.Percent.HasValue ? -1 : 1;
            }
            if (left.Percent.HasValue && right.Percent.HasValue)
            {
                int percentComparison = left.Percent.Value.CompareTo(right.Percent.Value);
                if (percentComparison != 0)
                {
                    return percentComparison;
                }
            }
            return string.Compare(
                left.Name,
                right.Name,
                StringComparison.CurrentCultureIgnoreCase);
        }

        private static LimitWindow CreateWindow(BatteryProbeResult device)
        {
            LimitWindow window = new LimitWindow();
            window.DeviceId = device.Id;
            window.DisplayName = device.Name;
            window.BatteryPercent = device.Percent;
            window.UsedPercent = device.Percent.HasValue
                ? 100.0 - device.Percent.Value
                : 100.0;
            window.DetailLeft = device.Source;
            window.DetailRight = device.Status;
            window.IsCached = device.IsCached;
            window.BatteryEstimate = device.BatteryEstimate;
            return window;
        }
    }
}
