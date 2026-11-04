using System;
using System.Collections.Generic;
using System.Reflection;
using CodexUsageTray;

internal static class BatteryMonitorClientTests
{
    private static readonly MethodInfo Merge = typeof(BatteryMonitorClient).GetMethod(
        "MergeAndCache", BindingFlags.NonPublic | BindingFlags.Static);

    private static void Main()
    {
        BatteryMonitorClient.InitializeBatteryCache(null);
        BatteryProbeResult first = Device("identity-a", "Same model", "Logitech", 40);
        BatteryProbeResult second = Device("identity-b", "Same model", "Logitech", 80);
        List<BatteryProbeResult> distinct = Run(first, second);
        Assert(distinct.Count == 2, "same-source devices with the same name remain separate");

        BatteryMonitorClient.InitializeBatteryCache(null);
        BatteryProbeResult duplicate = Device("identity-c", "One device", "Windows", 60);
        BatteryProbeResult otherSource = Device("identity-d", "One device", "Bluetooth LE", 60);
        Assert(Run(duplicate, otherSource).Count == 1, "unambiguous sources are merged");

        BatteryMonitorClient.InitializeBatteryCache(null);
        Run(Device("identity-a", "Same model", "Logitech", 40));
        BatteryProbeResult otherId = Device("identity-e", "Same model", "Logitech", null);
        Assert(!Find(Run(otherId), "identity-e").Percent.HasValue, "cache does not use a different device's name");
        BatteryProbeResult sameId = Device("identity-a", "Changed name", "Logitech", null);
        BatteryProbeResult cached = Find(Run(sameId), "identity-a");
        Assert(cached.IsCached && cached.Percent == 40, "cache uses matching device ID");

        long revision;
        List<CachedBatteryReading> saved;
        Assert(BatteryMonitorClient.TryGetUpdatedBatteryCache(0, out revision, out saved) && saved.Count == 1,
            "fresh battery observations are exported for persistence");
        DateTime observedAt = saved[0].ObservedAt;
        BatteryMonitorClient.InitializeBatteryCache(saved);
        List<BatteryProbeResult> afterRestart = Run();
        Assert(afterRestart.Count == 1 && afterRestart[0].Id == "identity-a" && afterRestart[0].IsCached &&
            afterRestart[0].Percent == 40 && afterRestart[0].ObservedAt == observedAt &&
            afterRestart[0].Status.Contains("Last reported"),
            "a missing device retains its last value and observation time after restart");
        Assert(!BatteryMonitorClient.TryGetUpdatedBatteryCache(0, out revision, out saved),
            "cached observations do not trigger fresh writes");
        BatteryProbeResult fresh = Find(Run(Device("identity-a", "Same model", "Logitech", 35)), "identity-a");
        Assert(!fresh.IsCached && fresh.Percent == 35, "fresh readings replace stale values");
        Assert(BatteryMonitorClient.TryGetUpdatedBatteryCache(0, out revision, out saved) && saved[0].Percent == 35,
            "changed live readings trigger persistence");
        BatteryRateTracker tracker = new BatteryRateTracker();
        tracker.Update(new List<BatteryProbeResult> { afterRestart[0] }, 1);
        Assert(afterRestart[0].BatteryEstimate == null && tracker.HistoryRevision == 0,
            "cached readings do not contribute to rate learning");

        DateTime yesterday = DateTime.Now.Date.AddDays(-1).AddHours(8);
        BatteryMonitorClient.InitializeBatteryCache(new List<CachedBatteryReading>
        {
            new CachedBatteryReading { Id = "windows-id", Name = "Headset", Source = "Windows", Percent = 48,
                Priority = 10, ObservedAt = yesterday },
            new CachedBatteryReading { Id = "ble-id", Name = "Headset", Source = "Bluetooth LE", Percent = 50,
                Priority = 20, ObservedAt = yesterday }
        });
        List<BatteryProbeResult> absentSources = Run();
        Assert(absentSources.Count == 1 && absentSources[0].IsCached &&
            absentSources[0].Status.Contains(yesterday.ToString("d", System.Globalization.CultureInfo.CurrentCulture)),
            "missing cross-source entries merge and include their previous-day date");
        List<BatteryProbeResult> liveSource = Run(Device("ble-id", "Headset", "Bluetooth LE", 37));
        Assert(liveSource.Count == 1 && liveSource[0].Id == "ble-id" && !liveSource[0].IsCached && liveSource[0].Percent == 37,
            "a live source wins over missing cached entries for the same device");
        BatteryMonitorClient.InitializeBatteryCache(null);
        Console.WriteLine("Battery identity and cache tests passed.");
    }

    private static BatteryProbeResult Find(List<BatteryProbeResult> results, string id)
    {
        foreach (BatteryProbeResult result in results) if (result.Id == id) return result;
        throw new Exception("Device not found: " + id);
    }

    private static List<BatteryProbeResult> Run(params BatteryProbeResult[] values)
    {
        return (List<BatteryProbeResult>)Merge.Invoke(null, new object[]
        {
            new List<BatteryProbeResult>(values), DateTime.Now
        });
    }

    private static BatteryProbeResult Device(string id, string name, string source, int? percent)
    {
        return new BatteryProbeResult { Id = id, Name = name, Source = source, Percent = percent };
    }

    private static void Assert(bool condition, string description)
    {
        if (!condition)
        {
            throw new Exception(description);
        }
    }
}
