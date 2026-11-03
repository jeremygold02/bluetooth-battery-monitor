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
        BatteryProbeResult first = Device("identity-a", "Same model", "Logitech", 40);
        BatteryProbeResult second = Device("identity-b", "Same model", "Logitech", 80);
        List<BatteryProbeResult> distinct = Run(first, second);
        Assert(distinct.Count == 2, "same-source devices with the same name remain separate");

        BatteryProbeResult duplicate = Device("identity-c", "One device", "Windows", 60);
        BatteryProbeResult otherSource = Device("identity-d", "One device", "Bluetooth LE", 60);
        Assert(Run(duplicate, otherSource).Count == 1, "unambiguous sources are merged");

        BatteryProbeResult otherId = Device("identity-e", "Same model", "Logitech", null);
        Assert(!Run(otherId)[0].Percent.HasValue, "cache does not use a different device's name");
        BatteryProbeResult sameId = Device("identity-a", "Changed name", "Logitech", null);
        BatteryProbeResult cached = Run(sameId)[0];
        Assert(cached.IsCached && cached.Percent == 40, "cache uses matching device ID");
        Console.WriteLine("Battery identity and cache tests passed.");
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
