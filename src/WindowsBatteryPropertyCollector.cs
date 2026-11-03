using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace CodexUsageTray
{
    internal static class WindowsBatteryPropertyCollector
    {
        private const uint DigcfPresent = 0x00000002;
        private const uint DigcfAllClasses = 0x00000004;

        private static readonly DevPropKey BatteryLifeKey = new DevPropKey(
            new Guid("49CD1F76-5626-4B17-A4E8-18B4AA1A2213"),
            10);
        private static readonly DevPropKey FriendlyNameKey = new DevPropKey(
            new Guid("A45C254E-DF1C-4EFD-8020-67D146A850E0"),
            14);
        private static readonly DevPropKey DeviceDescriptionKey = new DevPropKey(
            new Guid("A45C254E-DF1C-4EFD-8020-67D146A850E0"),
            2);

        [StructLayout(LayoutKind.Sequential)]
        private struct SpDevInfoData
        {
            public int Size;
            public Guid ClassGuid;
            public uint DevInst;
            public IntPtr Reserved;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct DevPropKey
        {
            public Guid FormatId;
            public uint PropertyId;

            public DevPropKey(Guid formatId, uint propertyId)
            {
                FormatId = formatId;
                PropertyId = propertyId;
            }
        }

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr SetupDiGetClassDevs(
            IntPtr classGuid,
            string enumerator,
            IntPtr parent,
            uint flags);

        [DllImport("setupapi.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetupDiEnumDeviceInfo(
            IntPtr deviceInfoSet,
            uint memberIndex,
            ref SpDevInfoData deviceInfoData);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetupDiGetDeviceProperty(
            IntPtr deviceInfoSet,
            ref SpDevInfoData deviceInfoData,
            ref DevPropKey propertyKey,
            out uint propertyType,
            byte[] propertyBuffer,
            uint propertyBufferSize,
            out uint requiredSize,
            uint flags);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetupDiGetDeviceInstanceId(
            IntPtr deviceInfoSet,
            ref SpDevInfoData deviceInfoData,
            StringBuilder deviceInstanceId,
            int deviceInstanceIdSize,
            out int requiredSize);

        [DllImport("setupapi.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetupDiDestroyDeviceInfoList(IntPtr deviceInfoSet);

        public static List<BatteryProbeResult> Collect()
        {
            List<BatteryProbeResult> results = new List<BatteryProbeResult>();
            IntPtr deviceInfoSet = SetupDiGetClassDevs(
                IntPtr.Zero,
                null,
                IntPtr.Zero,
                DigcfPresent | DigcfAllClasses);
            if (deviceInfoSet == new IntPtr(-1))
            {
                return results;
            }

            try
            {
                for (uint index = 0; ; index++)
                {
                    SpDevInfoData data = new SpDevInfoData();
                    data.Size = Marshal.SizeOf(typeof(SpDevInfoData));
                    if (!SetupDiEnumDeviceInfo(deviceInfoSet, index, ref data))
                    {
                        break;
                    }

                    byte battery;
                    if (!TryGetByte(deviceInfoSet, ref data, BatteryLifeKey, out battery) ||
                        battery > 100)
                    {
                        continue;
                    }

                    string instanceId = GetInstanceId(deviceInfoSet, ref data);
                    if (!IsWirelessDeviceInstance(instanceId))
                    {
                        continue;
                    }

                    string name = GetString(deviceInfoSet, ref data, FriendlyNameKey);
                    if (string.IsNullOrWhiteSpace(name))
                    {
                        name = GetString(deviceInfoSet, ref data, DeviceDescriptionKey);
                    }

                    BatteryProbeResult result = new BatteryProbeResult();
                    result.Id = "windows:" + instanceId;
                    result.Name = name;
                    result.Percent = battery;
                    result.Source = "Windows";
                    result.Status = "Connected";
                    result.Priority = 10;
                    result.ObservedAt = DateTime.Now;
                    results.Add(result);
                }
            }
            finally
            {
                SetupDiDestroyDeviceInfoList(deviceInfoSet);
            }

            return results;
        }

        private static bool IsWirelessDeviceInstance(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
            {
                return false;
            }

            return value.StartsWith("BTH", StringComparison.OrdinalIgnoreCase) ||
                value.StartsWith("HID", StringComparison.OrdinalIgnoreCase) ||
                value.StartsWith("USB", StringComparison.OrdinalIgnoreCase) ||
                value.StartsWith("SWD", StringComparison.OrdinalIgnoreCase);
        }

        private static bool TryGetByte(
            IntPtr deviceInfoSet,
            ref SpDevInfoData data,
            DevPropKey key,
            out byte value)
        {
            value = 0;
            byte[] buffer = new byte[8];
            uint propertyType;
            uint requiredSize;
            if (!SetupDiGetDeviceProperty(
                deviceInfoSet,
                ref data,
                ref key,
                out propertyType,
                buffer,
                (uint)buffer.Length,
                out requiredSize,
                0) ||
                requiredSize < 1)
            {
                return false;
            }

            value = buffer[0];
            return true;
        }

        private static string GetString(
            IntPtr deviceInfoSet,
            ref SpDevInfoData data,
            DevPropKey key)
        {
            byte[] buffer = new byte[2048];
            uint propertyType;
            uint requiredSize;
            if (!SetupDiGetDeviceProperty(
                deviceInfoSet,
                ref data,
                ref key,
                out propertyType,
                buffer,
                (uint)buffer.Length,
                out requiredSize,
                0) ||
                requiredSize < 2)
            {
                return "";
            }

            int length = Math.Min(buffer.Length, (int)requiredSize);
            return Encoding.Unicode.GetString(buffer, 0, length).TrimEnd('\0');
        }

        private static string GetInstanceId(
            IntPtr deviceInfoSet,
            ref SpDevInfoData data)
        {
            StringBuilder value = new StringBuilder(1024);
            int requiredSize;
            return SetupDiGetDeviceInstanceId(
                deviceInfoSet,
                ref data,
                value,
                value.Capacity,
                out requiredSize)
                    ? value.ToString()
                    : Guid.NewGuid().ToString("N");
        }
    }
}
