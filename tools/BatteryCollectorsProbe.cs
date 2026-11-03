using System;
using System.Collections;
using System.IO;
using System.Reflection;

internal static class BatteryCollectorsProbe
{
    public static int Main()
    {
        string executable = Path.GetFullPath(
            Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "..", "BluetoothBatteryMonitor.exe"));
        Assembly assembly = Assembly.LoadFrom(executable);
        Type clientType = assembly.GetType("CodexUsageTray.BatteryMonitorClient", true);
        MethodInfo fetch = clientType.GetMethod(
            "FetchUsage",
            BindingFlags.Public | BindingFlags.Static);
        object snapshot = fetch.Invoke(null, null);

        Type snapshotType = snapshot.GetType();
        string status = ReadField(snapshotType, snapshot, "StatusMessage") as string;
        IEnumerable devices = ReadField(snapshotType, snapshot, "Devices") as IEnumerable;
        int count = 0;
        if (devices != null)
        {
            foreach (object device in devices)
            {
                Type deviceType = device.GetType();
                Console.WriteLine(
                    "DEVICE={0} BATTERY={1} SOURCE={2} STATUS={3} ID={4}",
                    ReadField(deviceType, device, "DisplayName") ?? "",
                    ReadField(deviceType, device, "BatteryPercent") ?? "unknown",
                    ReadField(deviceType, device, "DetailLeft") ?? "",
                    ReadField(deviceType, device, "DetailRight") ?? "",
                    ReadField(deviceType, device, "DeviceId") ?? "");
                count++;
            }
        }

        Console.WriteLine("COUNT=" + count);
        if (!string.IsNullOrWhiteSpace(status))
        {
            Console.WriteLine("STATUS=" + status);
        }
        return count > 0 ? 0 : 3;
    }

    private static object ReadField(Type type, object target, string name)
    {
        FieldInfo field = type.GetField(
            name,
            BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
        return field != null ? field.GetValue(target) : null;
    }
}
