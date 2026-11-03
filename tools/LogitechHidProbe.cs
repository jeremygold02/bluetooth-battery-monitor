using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

internal static class LogitechHidProbe
{
    private const uint DigcfPresent = 0x00000002;
    private const uint DigcfDeviceInterface = 0x00000010;
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareRead = 0x00000001;
    private const uint FileShareWrite = 0x00000002;
    private const uint OpenExisting = 3;
    private const uint FileFlagOverlapped = 0x40000000;

    [StructLayout(LayoutKind.Sequential)]
    private struct SpDeviceInterfaceData
    {
        public int Size;
        public Guid InterfaceClassGuid;
        public int Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HidAttributes
    {
        public int Size;
        public ushort VendorId;
        public ushort ProductId;
        public ushort VersionNumber;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HidpCaps
    {
        public ushort Usage;
        public ushort UsagePage;
        public ushort InputReportByteLength;
        public ushort OutputReportByteLength;
        public ushort FeatureReportByteLength;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = 17)]
        public ushort[] Reserved;
        public ushort NumberLinkCollectionNodes;
        public ushort NumberInputButtonCaps;
        public ushort NumberInputValueCaps;
        public ushort NumberInputDataIndices;
        public ushort NumberOutputButtonCaps;
        public ushort NumberOutputValueCaps;
        public ushort NumberOutputDataIndices;
        public ushort NumberFeatureButtonCaps;
        public ushort NumberFeatureValueCaps;
        public ushort NumberFeatureDataIndices;
    }

    [DllImport("hid.dll")]
    private static extern void HidD_GetHidGuid(out Guid hidGuid);

    [DllImport("setupapi.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevs(
        ref Guid classGuid,
        IntPtr enumerator,
        IntPtr parent,
        uint flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiEnumDeviceInterfaces(
        IntPtr deviceInfoSet,
        IntPtr deviceInfoData,
        ref Guid interfaceClassGuid,
        uint memberIndex,
        ref SpDeviceInterfaceData deviceInterfaceData);

    [DllImport("setupapi.dll", CharSet = CharSet.Auto, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiGetDeviceInterfaceDetail(
        IntPtr deviceInfoSet,
        ref SpDeviceInterfaceData deviceInterfaceData,
        IntPtr deviceInterfaceDetailData,
        uint deviceInterfaceDetailDataSize,
        out uint requiredSize,
        IntPtr deviceInfoData);

    [DllImport("setupapi.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr deviceInfoSet);

    [DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern SafeFileHandle CreateFile(
        string fileName,
        uint desiredAccess,
        uint shareMode,
        IntPtr securityAttributes,
        uint creationDisposition,
        uint flagsAndAttributes,
        IntPtr templateFile);

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_GetAttributes(
        SafeFileHandle hidDeviceObject,
        ref HidAttributes attributes);

    [DllImport("hid.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_GetManufacturerString(
        SafeFileHandle hidDeviceObject,
        IntPtr buffer,
        int bufferLength);

    [DllImport("hid.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_GetProductString(
        SafeFileHandle hidDeviceObject,
        IntPtr buffer,
        int bufferLength);

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_GetPreparsedData(
        SafeFileHandle hidDeviceObject,
        out IntPtr preparsedData);

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_FreePreparsedData(IntPtr preparsedData);

    [DllImport("hid.dll")]
    private static extern int HidP_GetCaps(IntPtr preparsedData, out HidpCaps capabilities);

    public static int Main()
    {
        List<string> paths = EnumerateHidPaths();
        int candidateCount = 0;

        foreach (string path in paths)
        {
            using (SafeFileHandle handle = Open(path))
            {
                if (handle.IsInvalid)
                {
                    continue;
                }

                HidAttributes attributes = new HidAttributes();
                attributes.Size = Marshal.SizeOf(typeof(HidAttributes));
                if (!HidD_GetAttributes(handle, ref attributes) || attributes.VendorId != 0x046D)
                {
                    continue;
                }

                HidpCaps caps;
                if (!TryGetCaps(handle, out caps))
                {
                    continue;
                }

                candidateCount++;
                Console.WriteLine("PATH=" + path);
                Console.WriteLine(
                    "VID={0:X4} PID={1:X4} USAGE_PAGE={2:X4} USAGE={3:X4} IN={4} OUT={5} FEATURE={6}",
                    attributes.VendorId,
                    attributes.ProductId,
                    caps.UsagePage,
                    caps.Usage,
                    caps.InputReportByteLength,
                    caps.OutputReportByteLength,
                    caps.FeatureReportByteLength);
                Console.WriteLine("MANUFACTURER=" + ReadString(handle, true));
                Console.WriteLine("PRODUCT=" + ReadString(handle, false));

                if (attributes.ProductId == 0xC53F &&
                    caps.InputReportByteLength >= 7 &&
                    caps.OutputReportByteLength >= 7)
                {
                    ProbeHidpp(path, caps);
                }

                Console.WriteLine();
            }
        }

        Console.WriteLine("LOGITECH_HID_CANDIDATES=" + candidateCount);
        return candidateCount > 0 ? 0 : 2;
    }

    private static void ProbeHidpp(string path, HidpCaps caps)
    {
        using (SafeFileHandle handle = Open(path))
        {
            if (handle.IsInvalid)
            {
                Console.WriteLine("HIDPP_OPEN=FAILED");
                return;
            }

            using (FileStream stream = new FileStream(
                handle,
                FileAccess.ReadWrite,
                Math.Max(64, (int)caps.InputReportByteLength),
                true))
            {
                for (byte deviceIndex = 1; deviceIndex <= 6; deviceIndex++)
                {
                    byte[] featureResponse;
                    if (!TryFeatureRequest(
                        stream,
                        caps,
                        deviceIndex,
                        0,
                        0x00,
                        new byte[] { 0x10, 0x04 },
                        out featureResponse))
                    {
                        continue;
                    }

                    byte batteryFeatureIndex = featureResponse[4];
                    Console.WriteLine(
                        "HIDPP_DEVICE_INDEX={0} UNIFIED_BATTERY_FEATURE_INDEX={1:X2}",
                        deviceIndex,
                        batteryFeatureIndex);

                    byte[] batteryResponse;
                    if (batteryFeatureIndex != 0 &&
                        TryFeatureRequest(
                            stream,
                            caps,
                            deviceIndex,
                            batteryFeatureIndex,
                            0x10,
                            new byte[0],
                            out batteryResponse))
                    {
                        Console.WriteLine(
                            "HIDPP_BATTERY_PERCENT={0} LEVEL_FLAGS={1:X2} STATUS={2:X2} EXTERNAL_POWER={3:X2}",
                            batteryResponse[4],
                            batteryResponse[5],
                            batteryResponse[6],
                            batteryResponse.Length > 7 ? batteryResponse[7] : (byte)0);
                    }

                    ProbeDeviceName(stream, caps, deviceIndex);
                }
            }
        }
    }

    private static void ProbeDeviceName(FileStream stream, HidpCaps caps, byte deviceIndex)
    {
        byte[] featureResponse;
        if (!TryFeatureRequest(
            stream,
            caps,
            deviceIndex,
            0,
            0x00,
            new byte[] { 0x00, 0x05 },
            out featureResponse))
        {
            return;
        }

        byte nameFeatureIndex = featureResponse[4];
        if (nameFeatureIndex == 0)
        {
            return;
        }

        byte[] countResponse;
        if (!TryFeatureRequest(
            stream,
            caps,
            deviceIndex,
            nameFeatureIndex,
            0x00,
            new byte[0],
            out countResponse))
        {
            return;
        }

        int nameLength = countResponse[4];
        if (nameLength <= 0 || nameLength > 64)
        {
            return;
        }

        List<byte> nameBytes = new List<byte>();
        int offset = 0;
        while (offset < nameLength)
        {
            byte[] fragmentResponse;
            if (!TryFeatureRequest(
                stream,
                caps,
                deviceIndex,
                nameFeatureIndex,
                0x10,
                new byte[] { (byte)offset },
                out fragmentResponse))
            {
                break;
            }

            int available = Math.Min(nameLength - offset, fragmentResponse.Length - 4);
            for (int index = 0; index < available; index++)
            {
                nameBytes.Add(fragmentResponse[4 + index]);
            }
            offset += available;
        }

        Console.WriteLine("HIDPP_DEVICE_NAME=" + System.Text.Encoding.UTF8.GetString(nameBytes.ToArray()).TrimEnd('\0'));
    }

    private static bool TryFeatureRequest(
        FileStream stream,
        HidpCaps caps,
        byte deviceIndex,
        byte featureIndex,
        byte function,
        byte[] parameters,
        out byte[] response)
    {
        response = null;
        int outputLength = caps.OutputReportByteLength;
        int inputLength = caps.InputReportByteLength;
        if (outputLength < 7 || inputLength < 7)
        {
            return false;
        }

        byte reportId = outputLength >= 20 ? (byte)0x11 : (byte)0x10;
        byte[] request = new byte[outputLength];
        request[0] = reportId;
        request[1] = deviceIndex;
        request[2] = featureIndex;
        request[3] = (byte)(function | 0x08);
        int parameterCount = Math.Min(parameters != null ? parameters.Length : 0, outputLength - 4);
        if (parameterCount > 0)
        {
            Buffer.BlockCopy(parameters, 0, request, 4, parameterCount);
        }

        DateTime deadline = DateTime.UtcNow.AddMilliseconds(1200);
        while (DateTime.UtcNow < deadline)
        {
            byte[] input = new byte[inputLength];
            IAsyncResult read = stream.BeginRead(input, 0, input.Length, null, null);
            stream.Write(request, 0, request.Length);
            stream.Flush();

            int remaining = Math.Max(1, (int)(deadline - DateTime.UtcNow).TotalMilliseconds);
            if (!read.AsyncWaitHandle.WaitOne(remaining))
            {
                return false;
            }

            int readCount;
            try
            {
                readCount = stream.EndRead(read);
            }
            catch (IOException)
            {
                return false;
            }

            if (readCount >= 7 &&
                input[1] == deviceIndex &&
                input[2] == featureIndex &&
                input[3] == (byte)(function | 0x08))
            {
                response = input;
                return true;
            }

            if (readCount >= 7 &&
                input[1] == deviceIndex &&
                input[2] == 0xFF &&
                input[3] == featureIndex &&
                input[4] == (byte)(function | 0x08))
            {
                return false;
            }
        }

        return false;
    }

    private static SafeFileHandle Open(string path)
    {
        return CreateFile(
            path,
            GenericRead | GenericWrite,
            FileShareRead | FileShareWrite,
            IntPtr.Zero,
            OpenExisting,
            FileFlagOverlapped,
            IntPtr.Zero);
    }

    private static bool TryGetCaps(SafeFileHandle handle, out HidpCaps caps)
    {
        caps = new HidpCaps();
        IntPtr preparsedData;
        if (!HidD_GetPreparsedData(handle, out preparsedData))
        {
            return false;
        }

        try
        {
            return HidP_GetCaps(preparsedData, out caps) >= 0;
        }
        finally
        {
            HidD_FreePreparsedData(preparsedData);
        }
    }

    private static string ReadString(SafeFileHandle handle, bool manufacturer)
    {
        IntPtr buffer = Marshal.AllocHGlobal(512);
        try
        {
            bool success = manufacturer
                ? HidD_GetManufacturerString(handle, buffer, 512)
                : HidD_GetProductString(handle, buffer, 512);
            return success ? Marshal.PtrToStringUni(buffer) : "";
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
    }

    private static List<string> EnumerateHidPaths()
    {
        Guid hidGuid;
        HidD_GetHidGuid(out hidGuid);
        IntPtr deviceInfoSet = SetupDiGetClassDevs(
            ref hidGuid,
            IntPtr.Zero,
            IntPtr.Zero,
            DigcfPresent | DigcfDeviceInterface);
        if (deviceInfoSet == new IntPtr(-1))
        {
            throw new InvalidOperationException("Could not enumerate HID interfaces.");
        }

        List<string> paths = new List<string>();
        try
        {
            for (uint index = 0; ; index++)
            {
                SpDeviceInterfaceData interfaceData = new SpDeviceInterfaceData();
                interfaceData.Size = Marshal.SizeOf(typeof(SpDeviceInterfaceData));
                if (!SetupDiEnumDeviceInterfaces(
                    deviceInfoSet,
                    IntPtr.Zero,
                    ref hidGuid,
                    index,
                    ref interfaceData))
                {
                    break;
                }

                uint requiredSize;
                SetupDiGetDeviceInterfaceDetail(
                    deviceInfoSet,
                    ref interfaceData,
                    IntPtr.Zero,
                    0,
                    out requiredSize,
                    IntPtr.Zero);
                if (requiredSize == 0)
                {
                    continue;
                }

                IntPtr detailData = Marshal.AllocHGlobal((int)requiredSize);
                try
                {
                    Marshal.WriteInt32(detailData, IntPtr.Size == 8 ? 8 : 6);
                    if (SetupDiGetDeviceInterfaceDetail(
                        deviceInfoSet,
                        ref interfaceData,
                        detailData,
                        requiredSize,
                        out requiredSize,
                        IntPtr.Zero))
                    {
                        string path = Marshal.PtrToStringAuto(IntPtr.Add(detailData, 4));
                        if (!string.IsNullOrWhiteSpace(path))
                        {
                            paths.Add(path);
                        }
                    }
                }
                finally
                {
                    Marshal.FreeHGlobal(detailData);
                }
            }
        }
        finally
        {
            SetupDiDestroyDeviceInfoList(deviceInfoSet);
        }

        return paths;
    }
}
