using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;

namespace CodexUsageTray
{
    internal static class HyperxHidBatteryCollector
    {
        private const ushort HyperxVendorId = 0x03F0;
        private const ushort CloudIIISProductId = 0x02CC;
        private const ushort CloudIIISAlternateProductId = 0x06BE;
        private const ushort CommandUsagePage = 0x01C0;
        private const ushort CommandUsage = 0x0001;

        public static List<BatteryProbeResult> Collect()
        {
            return Collect(null);
        }

        public static List<BatteryProbeResult> Collect(Action<Exception> onDeviceError)
        {
            List<BatteryProbeResult> results = new List<BatteryProbeResult>();
            List<HidDeviceInfo> interfaces = HidDeviceEnumerator.Enumerate();
            for (int index = 0; index < interfaces.Count; index++)
            {
                HidDeviceInfo device = interfaces[index];
                if (device.VendorId != HyperxVendorId ||
                    (device.ProductId != CloudIIISProductId &&
                        device.ProductId != CloudIIISAlternateProductId) ||
                    device.UsagePage != CommandUsagePage ||
                    device.Usage != CommandUsage)
                {
                    continue;
                }

                BatteryProbeResult result = new BatteryProbeResult();
                result.Id = "hyperx:" + device.Path;
                result.Name = !string.IsNullOrWhiteSpace(device.ProductName)
                    ? device.ProductName
                    : "HyperX Cloud III S Wireless";
                result.Source = "HyperX 2.4 GHz";
                result.Priority = 30;
                result.ObservedAt = DateTime.Now;

                try
                {
                    using (HidDeviceConnection connection = new HidDeviceConnection(device))
                    {
                        byte[] response = ReadResponse(connection, 0x06);
                        if (response == null || response.Length <= 6)
                        {
                            result.Status = "Headset unavailable";
                        }
                        else if (response[6] == 0xFF)
                        {
                            result.Status = "Headset off or unavailable";
                        }
                        else if (response[6] <= 100)
                        {
                            result.Percent = response[6];
                            result.Status = ReadChargingStatus(connection);
                        }
                        else
                        {
                            result.Status = "Unexpected battery response";
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
                    result.Status = result.Percent.HasValue ? "Connected" : "Headset unavailable";
                }

                results.Add(result);
            }

            return results;
        }

        private static byte[] ReadResponse(HidDeviceConnection connection, byte command)
        {
            byte[] request = new byte[64];
            request[0] = 0x0C;
            request[1] = 0x02;
            request[2] = 0x03;
            request[3] = 0x01;
            request[5] = command;

            return connection.Exchange(
                request,
                delegate(byte[] value)
                {
                    return value != null &&
                        value.Length > 6 &&
                        value[0] == 0x0C &&
                        value[5] == command;
                },
                1500,
                true);
        }

        private static string ReadChargingStatus(HidDeviceConnection connection)
        {
            try
            {
                return FormatChargingStatus(ReadResponse(connection, 0x48));
            }
            catch (Win32Exception)
            {
                return "Connected";
            }
            catch (IOException)
            {
                return "Connected";
            }
            catch (TimeoutException)
            {
                return "Connected";
            }
        }

        private static string FormatChargingStatus(byte[] response)
        {
            if (response == null || response.Length <= 6)
            {
                return "Connected";
            }

            switch (response[6])
            {
                case 0:
                    return "Not charging";
                case 1:
                    return "Charging";
                case 2:
                    return "Fully charged";
                default:
                    return "Connected";
            }
        }
    }
}
