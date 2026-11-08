using System;
using System.Collections.Generic;
using System.Globalization;
using CodexUsageTray;

internal static class BatteryRateTrackerTests
{
    private static void Main()
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        TestQualificationAndNoDoubleCounting();
        TestRestartAndDirectionalSafety();
        TestPhysicalIdentityAcrossConnections();
        TestRollingAverage();
        TestInterruptionsAndUnstableIdentity();
        TestBriefCachedAndMissingGaps();
        TestLongGapExpiresFromLastLiveObservation();
        TestFullAndErrorStatusesResetLiveSegments();
        TestImportValidationAndCopies();
        Console.WriteLine("Battery rate tests passed.");
    }

    private static void TestPhysicalIdentityAcrossConnections()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        BatteryProbeResult wireless = Device("receiver-slot", 90, "Discharging");
        wireless.PhysicalId = "logitech-unit:C09F0000000012345678";
        Check(tracker, wireless, 0, null);
        wireless.Percent = 88;
        Check(tracker, wireless, 900, null);
        BatteryProbeResult wired = Device("wired-device", 87, "Discharging");
        wired.PhysicalId = wireless.PhysicalId;
        tracker.Update(new[] { wired }, 1800);
        Assert(tracker.HistoryRevision == 1 && tracker.ExportHistory()[0].DeviceId == wireless.PhysicalId,
            "the same physical mouse retains its learning segment across connection paths");
        BatteryRateTracker restarted = new BatteryRateTracker();
        restarted.ImportHistory(tracker.ExportHistory());
        Check(restarted, wireless, 2000, "Avg. \u22126%/h \u00b7 ~14h 40m remaining");
        BatteryRateTracker upgraded = new BatteryRateTracker();
        upgraded.ImportHistory(new[] { History("receiver-slot", 0, 0, 6, 3600) });
        Check(upgraded, wireless, 0, "Avg. \u22126%/h \u00b7 ~14h 40m remaining");
        Assert(upgraded.HistoryRevision == 1 && upgraded.ExportHistory().Count == 1 &&
            upgraded.ExportHistory()[0].DeviceId == wireless.PhysicalId,
            "existing history migrates from the observed path to the physical identity");
    }

    private static void TestQualificationAndNoDoubleCounting()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        BatteryProbeResult mouse = Device("mouse", 50, "Discharging");
        Check(tracker, mouse, 0, null);
        mouse.Percent = 48;
        Check(tracker, mouse, 900, null);
        mouse.Percent = 47;
        Check(tracker, mouse, 1799, null);
        Assert(tracker.HistoryRevision == 0 && tracker.ExportHistory().Count == 0,
            "unqualified observations are not saved");
        Check(tracker, mouse, 1800, "Est. \u22126%/h \u00b7 ~7h 50m remaining");
        Assert(tracker.HistoryRevision == 1 && tracker.ExportHistory()[0].DischargeSamples.Count == 1,
            "first qualified segment is recorded once");
        Check(tracker, mouse, 1860, "Avg. \u22126%/h \u00b7 ~7h 50m remaining");
        Check(tracker, mouse, 2700, "Avg. \u22126%/h \u00b7 ~7h 50m remaining");
        Assert(tracker.HistoryRevision == 1 && tracker.ExportHistory()[0].DischargeSamples.Count == 1,
            "polling without a newly qualified segment does not count observations twice");
        mouse.Percent = 45;
        Check(tracker, mouse, 3600, "Avg. \u22126%/h \u00b7 ~7h 30m remaining");
        mouse.Percent = 44;
        Check(tracker, mouse, 3660, "Avg. \u22125.9%/h \u00b7 ~7h 27m remaining");
        List<LearnedBatteryRateSample> samples = tracker.ExportHistory()[0].DischargeSamples;
        Assert(tracker.HistoryRevision == 2 && samples.Count == 2 && samples[0].Percent == 3 &&
            samples[0].Seconds == 1800 && samples[1].Percent == 3 && samples[1].Seconds == 1860,
            "successive segments do not overlap");
        tracker.Reset();
        Check(tracker, mouse, 4000, "Avg. \u22125.9%/h \u00b7 ~7h 27m remaining");
        Assert(tracker.HistoryRevision == 2 && tracker.ExportHistory()[0].DischargeSamples.Count == 2,
            "reset keeps both the learned samples and their revision");

        BatteryRateTracker tooFew = new BatteryRateTracker();
        BatteryProbeResult slowPoll = Device("slow-poll", 60, "Discharging");
        Check(tooFew, slowPoll, 0, null);
        slowPoll.Percent = 57;
        Check(tooFew, slowPoll, 1800, null);
        Assert(tooFew.HistoryRevision == 0, "thirty minutes and three percent still need three readings");
        Check(tooFew, slowPoll, 1801, "Est. \u22126%/h \u00b7 ~9h 30m remaining");
        Assert(tooFew.HistoryRevision == 1, "the third reading qualifies the elapsed segment");
    }

    private static void TestRestartAndDirectionalSafety()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        tracker.ImportHistory(new[] { History("Headset", 12, 3600, 6, 3600) });
        Assert(tracker.HistoryRevision == 0, "import does not mark history as newly learned");
        BatteryProbeResult device = Device("headset", 60, "Discharging");
        Check(tracker, device, 0, "Avg. \u22126%/h \u00b7 ~10h 0m remaining");
        device.Status = "Charging";
        Check(tracker, device, 60, "Avg. +12%/h \u00b7 ~3h 20m to full");
        device.Status = "Connected";
        Check(tracker, device, 120, "Avg. use \u22126%/h");
        device.Status = "Not charging";
        Check(tracker, device, 180, "Avg. use \u22126%/h");
        device.IsCached = true;
        Check(tracker, device, 240, null);
        device.IsCached = false;
        device.Percent = null;
        Check(tracker, device, 300, null);
        device.Percent = 100;
        device.Status = "Fully charged";
        Check(tracker, device, 360, null);
        device.Status = "Charging error";
        Check(tracker, device, 420, null);
        device.Status = "Charging";
        Check(tracker, device, 480, "Avg. +12%/h");
        device.Percent = 0;
        device.Status = "Discharging";
        Check(tracker, device, 540, "Avg. \u22126%/h");

        BatteryRateTracker separate = new BatteryRateTracker();
        separate.ImportHistory(new[] { History("one-way", 0, 0, 6, 3600) });
        BatteryProbeResult other = Device("one-way", 60, "Charging");
        Check(separate, other, 0, null);
        other.Status = "Connected";
        Check(separate, other, 60, "Avg. use \u22126%/h");
        other.Percent = 61;
        Check(separate, other, 120, null);
        Assert(separate.HistoryRevision == 0, "wrong-direction samples do not create history");

        device = Device("headset", 60, "Not charging");
        tracker.Reset();
        Check(tracker, device, 600, "Avg. use \u22126%/h");
        device.Percent = 61;
        Check(tracker, device, 1200, null);
        device.Percent = 63;
        Check(tracker, device, 2400, null);
        Assert(tracker.HistoryRevision == 0, "a rise reported as not charging is not learned as charging");
    }

    private static void TestRollingAverage()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        LearnedBatteryRate initial = History("rolling", 3, 1800, 0, 0);
        initial.DischargeSamples.Add(Sample(6, 1800));
        for (int i = 1; i < 20; i++) initial.DischargeSamples.Add(Sample(3, 1800));
        tracker.ImportHistory(new[] { initial });
        BatteryProbeResult device = Device("rolling", 90, "Discharging");
        Check(tracker, device, 0, "Avg. \u22126.3%/h \u00b7 ~14h 17m remaining");
        device.Percent = 88;
        tracker.Update(new[] { device }, 1800);
        device.Percent = 87;
        tracker.Update(new[] { device }, 3600);
        List<LearnedBatteryRate> saved = tracker.ExportHistory();
        Assert(saved.Count == 1 && saved[0].DischargeSamples.Count == 20 && saved[0].ChargeSamples.Count == 1,
            "only the most recent twenty samples are retained separately by direction");
        Assert(saved[0].DischargeSamples[0].Percent == 3 && saved[0].DischargeSamples[19].Seconds == 3600,
            "the oldest sample is discarded when a new one qualifies");
        Assert(device.BatteryEstimate.StartsWith("Avg. \u22125.71%/h", StringComparison.Ordinal),
            "rolling average is weighted by observation duration");
        device.Percent = 85;
        tracker.Update(new[] { device }, 4500);
        device.Percent = 84;
        tracker.Update(new[] { device }, 5400);
        saved = tracker.ExportHistory();
        Assert(tracker.HistoryRevision == 2 && saved[0].DischargeSamples.Count == 20 &&
            saved[0].DischargeSamples[18].Seconds == 3600 && saved[0].DischargeSamples[19].Seconds == 1800,
            "subsequent qualified segments continue to evict the oldest observation");
        BatteryRateTracker restarted = new BatteryRateTracker();
        restarted.ImportHistory(saved);
        Check(restarted, Device("rolling", 84, "Discharging"), 0, device.BatteryEstimate);
        Check(restarted, Device("rolling", 87, "Charging"), 60, "Avg. +6%/h \u00b7 ~2h 10m to full");
        Assert(restarted.HistoryRevision == 0, "restart history is available without adding new observations");
    }

    private static void TestInterruptionsAndUnstableIdentity()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        BatteryProbeResult mouse = Device("mouse", 60, "Discharging");
        Check(tracker, mouse, 0, null);
        mouse.Percent = 58;
        Check(tracker, mouse, 900, null);
        mouse.IsCached = true;
        Check(tracker, mouse, 1200, null);
        mouse.IsCached = false;
        mouse.Percent = 57;
        Check(tracker, mouse, 1800, "Est. \u22126%/h \u00b7 ~9h 30m remaining");
        Assert(tracker.HistoryRevision == 1,
            "the third live observation qualifies despite a brief cached reading");
        tracker.Update(new BatteryProbeResult[0], 2400);
        Check(tracker, mouse, 2700, "Avg. \u22126%/h \u00b7 ~9h 30m remaining");
        Check(tracker, mouse, 10000, "Avg. \u22126%/h \u00b7 ~9h 30m remaining");
        mouse.Percent = 58;
        Check(tracker, mouse, 10060, null);
        Assert(tracker.HistoryRevision == 1 && tracker.ExportHistory()[0].DischargeSamples.Count == 1,
            "absence, a long gap, and contradictory direction do not add duplicate history");

        BatteryRateTracker generated = new BatteryRateTracker();
        BatteryProbeResult temporary = Device("device:" + Guid.NewGuid().ToString("N"), 50, "Discharging");
        Check(generated, temporary, 0, null);
        temporary.Percent = 48;
        Check(generated, temporary, 900, null);
        temporary.Percent = 47;
        Check(generated, temporary, 1800, "Est. \u22126%/h \u00b7 ~7h 50m remaining");
        Assert(generated.ExportHistory().Count == 0 && generated.HistoryRevision == 0,
            "a fabricated transient device ID is never saved");

        BatteryRateTracker saved = new BatteryRateTracker();
        saved.ImportHistory(new[] { History("mouse", 0, 0, 6, 3600) });
        long revision = saved.HistoryRevision;
        saved.Reset();
        Check(saved, Device("mouse", 50, "Discharging"), 0, "Avg. \u22126%/h \u00b7 ~8h 20m remaining");
        Assert(saved.HistoryRevision == revision && saved.ExportHistory().Count == 1,
            "resume resets live observations but keeps learned history");
    }

    private static void TestBriefCachedAndMissingGaps()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        BatteryProbeResult mouse = Device("brief-gap", 60, "Discharging");
        Check(tracker, mouse, 0, null);

        mouse.Percent = 55;
        mouse.IsCached = true;
        Check(tracker, mouse, 600, null);
        Assert(tracker.HistoryRevision == 0 && tracker.ExportHistory().Count == 0,
            "cached percentage changes never create learned history");

        tracker.Update(new BatteryProbeResult[0], 1200);
        mouse.IsCached = false;
        mouse.Percent = 58;
        Check(tracker, mouse, 1800, null);
        Assert(tracker.HistoryRevision == 0,
            "cached and missing observations do not count toward the live sample minimum");

        mouse.Percent = 57;
        tracker.Update(new List<BatteryProbeResult> { mouse }, 3600);
        List<LearnedBatteryRate> history = tracker.ExportHistory();
        Assert(mouse.BatteryEstimate.StartsWith("Est. \u22123%/h", StringComparison.Ordinal) &&
            tracker.HistoryRevision == 1 && history.Count == 1 &&
            history[0].DischargeSamples.Count == 1 && history[0].DischargeSamples[0].Percent == 3 &&
            history[0].DischargeSamples[0].Seconds == 3600,
            "live readings resume a brief gap using only live percentages and samples");
    }

    private static void TestLongGapExpiresFromLastLiveObservation()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        BatteryProbeResult mouse = Device("long-gap", 60, "Discharging");
        Check(tracker, mouse, 0, null);

        mouse.Percent = 50;
        mouse.IsCached = true;
        Check(tracker, mouse, 7200, null);
        tracker.Update(new BatteryProbeResult[0], 7201);
        Assert(tracker.HistoryRevision == 0 && tracker.ExportHistory().Count == 0,
            "cached observations cannot extend a live segment beyond the two-hour limit");

        mouse.IsCached = false;
        mouse.Percent = 57;
        Check(tracker, mouse, 7210, null);
        mouse.Percent = 56;
        Check(tracker, mouse, 7810, null);
        Assert(tracker.HistoryRevision == 0,
            "a device reappearing after an expired gap starts a fresh segment");
    }

    private static void TestFullAndErrorStatusesResetLiveSegments()
    {
        string[] terminalStatuses = { "Fully charged", "Battery error", "Temperature warning", "Charging error" };
        foreach (string status in terminalStatuses)
        {
            BatteryRateTracker tracker = new BatteryRateTracker();
            BatteryProbeResult device = Device("terminal-state", 60, "Discharging");
            Check(tracker, device, 0, null);

            device.Status = status;
            device.Percent = string.Equals(status, "Fully charged", StringComparison.Ordinal) ? 100 : (int?)null;
            Check(tracker, device, 600, null);

            device.Status = "Discharging";
            device.Percent = 57;
            Check(tracker, device, 1200, null);
            device.Percent = 56;
            Check(tracker, device, 1800, null);
            Assert(tracker.HistoryRevision == 0 && tracker.ExportHistory().Count == 0,
                "full and error states clear the live segment before reappearance");
        }
    }

    private static void TestImportValidationAndCopies()
    {
        BatteryRateTracker tracker = new BatteryRateTracker();
        LearnedBatteryRate malformed = History("mouse", 3, 1800, 0, 0);
        malformed.DischargeSamples.Add(null);
        malformed.DischargeSamples.Add(Sample(double.NaN, 1800));
        malformed.DischargeSamples.Add(Sample(3, double.PositiveInfinity));
        malformed.DischargeSamples.Add(Sample(-3, 1800));
        malformed.DischargeSamples.Add(Sample(3, 1799));
        malformed.DischargeSamples.Add(Sample(101, 1800));
        malformed.DischargeSamples.Add(Sample(3, 7 * 24 * 3600 + 1));
        malformed.DischargeSamples.Add(Sample(6, 3600));
        tracker.ImportHistory(new[] { null, malformed, History("MOUSE", 3, 1800, 3, 1800),
            History("device:" + Guid.NewGuid().ToString("N"), 0, 0, 3, 1800) });
        List<LearnedBatteryRate> exported = tracker.ExportHistory();
        Assert(exported.Count == 1 && exported[0].ChargeSamples.Count == 1 && exported[0].DischargeSamples.Count == 1 &&
            exported[0].DischargeSamples[0].Percent == 6,
            "malformed samples, duplicate IDs and fabricated IDs are rejected");
        malformed.DischargeSamples[7].Percent = 99;
        exported[0].DischargeSamples[0].Percent = 99;
        Assert(tracker.ExportHistory()[0].DischargeSamples[0].Percent == 6,
            "imported and exported history cannot mutate tracker state");
        LearnedBatteryRate oversized = History("long-history", 0, 0, 0, 0);
        for (int i = 0; i < 25; i++) oversized.DischargeSamples.Add(Sample(3, 1800 + i));
        tracker.ImportHistory(new[] { oversized });
        Assert(tracker.ExportHistory()[0].DischargeSamples.Count == 20 &&
            tracker.ExportHistory()[0].DischargeSamples[0].Seconds == 1805 &&
            tracker.ExportHistory()[0].DischargeSamples[19].Seconds == 1824,
            "import preserves only the twenty most recent valid samples");
        tracker.ImportHistory(null);
        Assert(tracker.ExportHistory().Count == 0 && tracker.HistoryRevision == 0, "empty history is accepted");
    }

    private static LearnedBatteryRate History(string id, double charge, double chargeSeconds, double discharge, double dischargeSeconds)
    {
        LearnedBatteryRate result = new LearnedBatteryRate { DeviceId = id };
        if (charge > 0) result.ChargeSamples.Add(Sample(charge, chargeSeconds));
        if (discharge > 0) result.DischargeSamples.Add(Sample(discharge, dischargeSeconds));
        return result;
    }

    private static LearnedBatteryRateSample Sample(double percent, double seconds)
    {
        return new LearnedBatteryRateSample { Percent = percent, Seconds = seconds };
    }

    private static BatteryProbeResult Device(string id, int percent, string status)
    {
        return new BatteryProbeResult { Id = id, Percent = percent, Status = status };
    }

    private static void Check(BatteryRateTracker tracker, BatteryProbeResult device, double now, string expected)
    {
        tracker.Update(new List<BatteryProbeResult> { device }, now);
        Assert(device.BatteryEstimate == expected, "expected '" + expected + "', got '" + device.BatteryEstimate + "'");
    }

    private static void Assert(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }
}
