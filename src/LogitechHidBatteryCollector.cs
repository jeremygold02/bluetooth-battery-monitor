using System;
using System.Collections.Generic;
using System.Text;

namespace CodexUsageTray
{
    internal static class LogitechHidBatteryCollector
    {
        private const ushort LogitechVendorId = 0x046D;
        private const ushort HidppUsagePage = 0xFF00;
        private const ushort HidppLongUsage = 0x0002;
        private const ushort BatteryStatusFeature = 0x1000;
        private const ushort UnifiedBatteryFeature = 0x1004;
        private const ushort DeviceNameFeature = 0x0005;

        public static List<BatteryProbeResult> Collect()
        {
            return Collect(null);
        }

        public static List<BatteryProbeResult> Collect(Action<Exception> onDeviceError)
        {
            List<BatteryProbeResult> results = new List<BatteryProbeResult>();
            List<HidDeviceInfo> interfaces = HidDeviceEnumerator.Enumerate();
            for (int interfaceIndex = 0; interfaceIndex < interfaces.Count; interfaceIndex++)
            {
                HidDeviceInfo device = interfaces[interfaceIndex];
                if (device.VendorId != LogitechVendorId ||
                    device.UsagePage != HidppUsagePage ||
                    device.Usage != HidppLongUsage ||
                    device.InputReportLength < 20 ||
                    device.OutputReportLength < 20)
                {
                    continue;
                }

                try
                {
                    using (HidDeviceConnection connection = new HidDeviceConnection(device))
                    {
                        bool anyResponse = false;
                        for (byte deviceIndex = 1; deviceIndex <= 6; deviceIndex++)
                        {
                            bool responded;
                            BatteryProbeResult result = ReadDevice(
                                connection,
                                device,
                                deviceIndex,
                                out responded);
                            if (!responded)
                            {
                                break;
                            }
                            anyResponse = true;
                            if (result != null)
                            {
                                results.Add(result);
                            }
                        }

                        if (!anyResponse)
                        {
                            BatteryProbeResult sleeping = new BatteryProbeResult();
                            sleeping.Id = CreateId(device, 1);
                            sleeping.Name = "Logitech wireless device";
                            sleeping.Source = "Logitech LIGHTSPEED";
                            sleeping.Status = "Sleeping or unavailable";
                            sleeping.Priority = 30;
                            sleeping.ObservedAt = DateTime.Now;
                            results.Add(sleeping);
                        }
                    }
                }
                catch (Exception ex)
                {
                    if (onDeviceError == null)
                    {
                        throw;
                    }
                    onDeviceError(ex);
                }
            }

            return results;
        }

        private static BatteryProbeResult ReadDevice(
            HidDeviceConnection connection,
            HidDeviceInfo receiver,
            byte deviceIndex,
            out bool responded)
        {
            responded = false;
            byte batteryStatus;
            ushort batteryFeature = BatteryStatusFeature;
            int? percent = ReadBattery(
                connection,
                deviceIndex,
                BatteryStatusFeature,
                0x00,
                out batteryStatus,
                ref responded);
            if (!responded)
            {
                return null;
            }
            if (!percent.HasValue)
            {
                batteryFeature = UnifiedBatteryFeature;
                percent = ReadBattery(
                    connection,
                    deviceIndex,
                    UnifiedBatteryFeature,
                    0x10,
                    out batteryStatus,
                    ref responded);
            }

            string name = ReadDeviceName(connection, deviceIndex, ref responded);
            if (!responded)
            {
                return null;
            }
            if (string.IsNullOrWhiteSpace(name))
            {
                name = "Logitech wireless device";
            }

            BatteryProbeResult result = new BatteryProbeResult();
            result.Id = CreateId(receiver, deviceIndex);
            result.Name = name;
            result.Percent = percent;
            result.Source = "Logitech LIGHTSPEED";
            result.Status = percent.HasValue
                ? FormatBatteryStatus(batteryFeature, batteryStatus)
                : "Sleeping or unavailable";
            result.Priority = 30;
            result.ObservedAt = DateTime.Now;
            return result;
        }

        private static int? ReadBattery(
            HidDeviceConnection connection,
            byte deviceIndex,
            ushort featureId,
            byte function,
            out byte status,
            ref bool responded)
        {
            status = 0;
            byte? featureIndex = GetFeatureIndex(
                connection,
                deviceIndex,
                featureId,
                ref responded);
            if (!featureIndex.HasValue)
            {
                return null;
            }

            byte[] response = Request(
                connection,
                deviceIndex,
                featureIndex.Value,
                function,
                new byte[0]);
            if (response == null)
            {
                return null;
            }
            responded = true;
            if (response.Length < 7 || IsProtocolError(response))
            {
                return null;
            }

            status = response[6];
            return response[4] <= 100 ? (int?)response[4] : null;
        }

        private static string ReadDeviceName(
            HidDeviceConnection connection,
            byte deviceIndex,
            ref bool responded)
        {
            byte? featureIndex = GetFeatureIndex(
                connection,
                deviceIndex,
                DeviceNameFeature,
                ref responded);
            if (!featureIndex.HasValue)
            {
                return null;
            }

            byte[] countResponse = Request(
                connection,
                deviceIndex,
                featureIndex.Value,
                0x00,
                new byte[0]);
            if (countResponse == null ||
                countResponse.Length < 5 ||
                IsProtocolError(countResponse))
            {
                return null;
            }
            responded = true;

            int nameLength = Math.Min(64, (int)countResponse[4]);
            if (nameLength <= 0)
            {
                return null;
            }

            List<byte> name = new List<byte>();
            for (int offset = 0; offset < nameLength; offset += 16)
            {
                byte[] fragment = Request(
                    connection,
                    deviceIndex,
                    featureIndex.Value,
                    0x10,
                    new byte[] { (byte)offset });
                if (fragment == null ||
                    fragment.Length <= 4 ||
                    IsProtocolError(fragment))
                {
                    break;
                }

                int count = Math.Min(nameLength - offset, fragment.Length - 4);
                for (int index = 0; index < count; index++)
                {
                    name.Add(fragment[4 + index]);
                }
            }

            return Encoding.UTF8.GetString(name.ToArray()).TrimEnd('\0').Trim();
        }

        private static byte? GetFeatureIndex(
            HidDeviceConnection connection,
            byte deviceIndex,
            ushort featureId,
            ref bool responded)
        {
            byte[] response = Request(
                connection,
                deviceIndex,
                0x00,
                0x00,
                new byte[]
                {
                    (byte)(featureId >> 8),
                    (byte)(featureId & 0xFF)
                });
            if (response == null)
            {
                return null;
            }
            responded = true;
            if (response.Length < 5 ||
                IsProtocolError(response) ||
                response[4] == 0)
            {
                return null;
            }
            return response[4];
        }

        private static byte[] Request(
            HidDeviceConnection connection,
            byte deviceIndex,
            byte featureIndex,
            byte function,
            byte[] parameters)
        {
            byte command = (byte)(function | 0x08);
            byte[] request = new byte[20];
            request[0] = 0x11;
            request[1] = deviceIndex;
            request[2] = featureIndex;
            request[3] = command;
            if (parameters != null && parameters.Length > 0)
            {
                Buffer.BlockCopy(
                    parameters,
                    0,
                    request,
                    4,
                    Math.Min(16, parameters.Length));
            }

            return connection.Exchange(
                request,
                delegate(byte[] response)
                {
                    return response != null &&
                        response.Length >= 7 &&
                        response[1] == deviceIndex &&
                        ((response[2] == featureIndex && response[3] == command) ||
                            (response[2] == 0xFF &&
                                response[3] == featureIndex &&
                                response[4] == command));
                },
                650,
                false);
        }

        private static bool IsProtocolError(byte[] response)
        {
            return response != null &&
                response.Length >= 5 &&
                response[2] == 0xFF;
        }

        private static string CreateId(HidDeviceInfo receiver, byte deviceIndex)
        {
            return "logitech:" + receiver.Path + ":" + deviceIndex.ToString();
        }

        private static string FormatBatteryStatus(ushort feature, byte status)
        {
            if (feature == UnifiedBatteryFeature)
            {
                switch (status)
                {
                    case 0:
                        return "Discharging";
                    case 1:
                        return "Charging";
                    case 2:
                        return "Charging slowly";
                    case 3:
                        return "Fully charged";
                    case 4:
                        return "Charging error";
                    default:
                        return "Connected";
                }
            }

            switch (status)
            {
                case 0:
                    return "Discharging";
                case 1:
                    return "Charging";
                case 2:
                    return "Nearly charged";
                case 3:
                    return "Fully charged";
                case 4:
                    return "Charging slowly";
                case 5:
                    return "Battery error";
                case 6:
                    return "Temperature warning";
                case 7:
                    return "Charging error";
                default:
                    return "Connected";
            }
        }
    }
}
