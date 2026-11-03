using System;
using System.Collections.Generic;
using System.Runtime.InteropServices.WindowsRuntime;
using Windows.Devices.Bluetooth;
using Windows.Devices.Bluetooth.GenericAttributeProfile;
using Windows.Devices.Enumeration;
using Windows.Storage.Streams;

namespace CodexUsageTray
{
    internal static class BluetoothGattBatteryCollector
    {
        private static readonly Guid BatteryLevelStatusUuid =
            new Guid("00002bed-0000-1000-8000-00805f9b34fb");

        public static List<BatteryProbeResult> Collect()
        {
            return Collect(null);
        }

        public static List<BatteryProbeResult> Collect(Action<Exception> onDeviceError)
        {
            List<BatteryProbeResult> results = new List<BatteryProbeResult>();
            string selector = BluetoothLEDevice.GetDeviceSelectorFromConnectionStatus(
                BluetoothConnectionStatus.Connected);
            DeviceInformationCollection devices = DeviceInformation.FindAllAsync(selector)
                .AsTask()
                .GetAwaiter()
                .GetResult();

            for (int index = 0; index < devices.Count; index++)
            {
                DeviceInformation information = devices[index];
                try
                {
                    BluetoothLEDevice device = BluetoothLEDevice.FromIdAsync(information.Id)
                        .AsTask()
                        .GetAwaiter()
                        .GetResult();
                    if (device == null)
                    {
                        continue;
                    }

                    try
                    {
                        string status;
                        int? percent = ReadBatteryLevel(device, out status);
                        if (!percent.HasValue)
                        {
                            continue;
                        }

                        BatteryProbeResult result = new BatteryProbeResult();
                        result.Id = "ble:" + information.Id;
                        result.Name = !string.IsNullOrWhiteSpace(device.Name)
                            ? device.Name
                            : information.Name;
                        result.Percent = percent;
                        result.Source = "Bluetooth LE";
                        result.Status = status;
                        result.Priority = 20;
                        result.ObservedAt = DateTime.Now;
                        results.Add(result);
                    }
                    finally
                    {
                        device.Dispose();
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

        private static int? ReadBatteryLevel(BluetoothLEDevice device, out string status)
        {
            status = "Connected";
            GattDeviceServicesResult servicesResult = device.GetGattServicesForUuidAsync(
                GattServiceUuids.Battery,
                BluetoothCacheMode.Uncached)
                .AsTask()
                .GetAwaiter()
                .GetResult();
            if (servicesResult.Status != GattCommunicationStatus.Success)
            {
                return null;
            }

            for (int serviceIndex = 0;
                serviceIndex < servicesResult.Services.Count;
                serviceIndex++)
            {
                GattDeviceService service = servicesResult.Services[serviceIndex];
                try
                {
                    GattCharacteristicsResult characteristics =
                        service.GetCharacteristicsForUuidAsync(
                            GattCharacteristicUuids.BatteryLevel,
                            BluetoothCacheMode.Uncached)
                        .AsTask()
                        .GetAwaiter()
                        .GetResult();
                    if (characteristics.Status != GattCommunicationStatus.Success)
                    {
                        continue;
                    }

                    for (int characteristicIndex = 0;
                        characteristicIndex < characteristics.Characteristics.Count;
                        characteristicIndex++)
                    {
                        GattReadResult read = characteristics.Characteristics[characteristicIndex]
                            .ReadValueAsync(BluetoothCacheMode.Uncached)
                            .AsTask()
                            .GetAwaiter()
                            .GetResult();
                        if (read.Status != GattCommunicationStatus.Success ||
                            read.Value == null ||
                            read.Value.Length < 1)
                        {
                            continue;
                        }

                        using (DataReader reader = DataReader.FromBuffer(read.Value))
                        {
                            int level = reader.ReadByte();
                            if (level > 100)
                            {
                                continue;
                            }
                            status = ReadBatteryLevelStatus(service);
                            return level;
                        }
                    }
                }
                finally
                {
                    service.Dispose();
                }
            }

            return null;
        }

        private static string ReadBatteryLevelStatus(GattDeviceService service)
        {
            // Battery Level Status is optional; its absence or failure must not
            // discard the already successful Battery Level reading.
            try
            {
                GattCharacteristicsResult characteristics = service.GetCharacteristicsForUuidAsync(
                    BatteryLevelStatusUuid, BluetoothCacheMode.Uncached)
                    .AsTask()
                    .GetAwaiter()
                    .GetResult();
                if (characteristics.Status != GattCommunicationStatus.Success)
                {
                    return "Connected";
                }

                foreach (GattCharacteristic characteristic in characteristics.Characteristics)
                {
                    GattReadResult read = characteristic.ReadValueAsync(BluetoothCacheMode.Uncached)
                        .AsTask()
                        .GetAwaiter()
                        .GetResult();
                    if (read.Status != GattCommunicationStatus.Success || read.Value == null ||
                        read.Value.Length < 3)
                    {
                        continue;
                    }
                    using (DataReader reader = DataReader.FromBuffer(read.Value))
                    {
                        byte[] value = new byte[read.Value.Length];
                        reader.ReadBytes(value);
                        return FormatBatteryLevelStatus(value);
                    }
                }
            }
            catch (Exception)
            {
                // A device may expose the optional characteristic without granting read access.
            }
            return "Connected";
        }

        private static string FormatBatteryLevelStatus(byte[] value)
        {
            if (value == null || value.Length < 3)
            {
                return "Connected";
            }

            int flags = value[0];
            int requiredLength = 3 + ((flags & 1) != 0 ? 2 : 0) +
                ((flags & 2) != 0 ? 1 : 0) + ((flags & 4) != 0 ? 1 : 0);
            if (value.Length < requiredLength || (value[1] & 1) == 0)
            {
                return "Connected";
            }

            // The two-byte Power State field is little-endian. Its charge state
            // occupies bits 5–6; inactive discharge is not active use.
            switch ((value[1] >> 5) & 3)
            {
                case 1:
                    return "Charging";
                case 2:
                    return "Discharging";
                case 3:
                    return "Not charging";
                default:
                    return "Connected";
            }
        }
    }
}
