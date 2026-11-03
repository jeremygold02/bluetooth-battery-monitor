using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

internal static class HyperxHidProbe
{
    private const ushort HyperxVendorId = 0x03F0;
    private const ushort CloudIIISProductId = 0x02CC;
    private const ushort CommandUsagePage = 0x01C0;
    private const ushort CommandUsage = 0x0001;

    private const uint DigcfPresent = 0x00000002;
    private const uint DigcfDeviceInterface = 0x00000010;
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareRead = 0x00000001;
    private const uint FileShareWrite = 0x00000002;
    private const uint OpenExisting = 3;
    private const uint FileFlagOverlapped = 0x40000000;
    private const int ErrorIoPending = 997;
    private const int ErrorInvalidFunction = 1;
    private const uint WaitObject0 = 0;

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

    [StructLayout(LayoutKind.Sequential)]
    private struct Overlapped
    {
        public IntPtr Internal;
        public IntPtr InternalHigh;
        public uint Offset;
        public uint OffsetHigh;
        public IntPtr EventHandle;
    }

    private sealed class HidInterface
    {
        public string Path;
        public HidpCaps Caps;
        public string Product;
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

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ReadFile(
        SafeFileHandle file,
        byte[] buffer,
        int numberOfBytesToRead,
        out int numberOfBytesRead,
        ref Overlapped overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool WriteFile(
        SafeFileHandle file,
        byte[] buffer,
        int numberOfBytesToWrite,
        out int numberOfBytesWritten,
        ref Overlapped overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetOverlappedResult(
        SafeFileHandle file,
        ref Overlapped overlapped,
        out int numberOfBytesTransferred,
        bool wait);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CancelIoEx(SafeFileHandle file, ref Overlapped overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr CreateEvent(
        IntPtr eventAttributes,
        bool manualReset,
        bool initialState,
        string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr handle);

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_GetAttributes(
        SafeFileHandle hidDeviceObject,
        ref HidAttributes attributes);

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

    [DllImport("hid.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool HidD_SetFeature(
        SafeFileHandle hidDeviceObject,
        byte[] reportBuffer,
        int reportBufferLength);

    public static int Main()
    {
        List<HidInterface> interfaces = EnumerateHyperxInterfaces();
        foreach (HidInterface item in interfaces)
        {
            Console.WriteLine(
                "USAGE_PAGE={0:X4} USAGE={1:X4} IN={2} OUT={3} FEATURE={4} PRODUCT={5}",
                item.Caps.UsagePage,
                item.Caps.Usage,
                item.Caps.InputReportByteLength,
                item.Caps.OutputReportByteLength,
                item.Caps.FeatureReportByteLength,
                item.Product);
            Console.WriteLine("PATH=" + item.Path);
        }

        HidInterface commandInterface = interfaces.Find(
            item => item.Caps.UsagePage == CommandUsagePage &&
                    item.Caps.Usage == CommandUsage);
        if (commandInterface == null)
        {
            Console.Error.WriteLine("Cloud III S command interface was not found.");
            return 2;
        }

        Console.WriteLine("SELECTED_PATH=" + commandInterface.Path);
        byte[] response = QueryBattery(commandInterface);
        Console.WriteLine("RESPONSE=" + Format(response));
        if (response == null || response.Length <= 6)
        {
            return 3;
        }

        int battery = response[6];
        Console.WriteLine("BATTERY_RAW=" + battery);
        if (battery == 0xFF)
        {
            Console.WriteLine("HEADSET_STATE=off-or-unavailable");
            return 4;
        }
        if (battery > 100)
        {
            Console.Error.WriteLine("Unexpected battery value.");
            return 5;
        }

        Console.WriteLine("BATTERY_PERCENT=" + battery);
        return 0;
    }

    private static byte[] QueryBattery(HidInterface item)
    {
        using (SafeFileHandle handle = CreateFile(
            item.Path,
            GenericRead | GenericWrite,
            FileShareRead | FileShareWrite,
            IntPtr.Zero,
            OpenExisting,
            FileFlagOverlapped,
            IntPtr.Zero))
        {
            if (handle.IsInvalid)
            {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(),
                    "Could not open Cloud III S HID interface.");
            }

            int responseLength = Math.Max(20, (int)item.Caps.InputReportByteLength);
            byte[] response = new byte[responseLength];
            Overlapped readOverlapped = CreateOverlapped();
            try
            {
                int ignored;
                bool readStarted = ReadFile(
                    handle,
                    response,
                    response.Length,
                    out ignored,
                    ref readOverlapped);
                int readError = Marshal.GetLastWin32Error();
                if (!readStarted && readError != ErrorIoPending)
                {
                    throw new Win32Exception(readError, "Could not start Cloud III S HID read.");
                }

                int packetLength = Math.Max(
                    64,
                    Math.Max(
                        (int)item.Caps.OutputReportByteLength,
                        (int)item.Caps.FeatureReportByteLength));
                byte[] request = new byte[packetLength];
                request[0] = 0x0C;
                request[1] = 0x02;
                request[2] = 0x03;
                request[3] = 0x01;
                request[4] = 0x00;
                request[5] = 0x06;

                try
                {
                    Write(handle, request);
                    Console.WriteLine("WRITE_MODE=output-report");
                }
                catch (Win32Exception ex)
                {
                    if (ex.NativeErrorCode != ErrorInvalidFunction ||
                        !HidD_SetFeature(handle, request, request.Length))
                    {
                        throw;
                    }
                    Console.WriteLine("WRITE_MODE=feature-report");
                }

                if (WaitForSingleObject(readOverlapped.EventHandle, 1500) != WaitObject0)
                {
                    CancelIoEx(handle, ref readOverlapped);
                    WaitForSingleObject(readOverlapped.EventHandle, 250);
                    return null;
                }

                int readCount;
                if (!GetOverlappedResult(handle, ref readOverlapped, out readCount, false))
                {
                    return null;
                }

                if (readCount != response.Length)
                {
                    Array.Resize(ref response, readCount);
                }
                return response;
            }
            finally
            {
                CloseHandle(readOverlapped.EventHandle);
            }
        }
    }

    private static void Write(SafeFileHandle handle, byte[] request)
    {
        Overlapped writeOverlapped = CreateOverlapped();
        try
        {
            int ignored;
            bool completed = WriteFile(
                handle,
                request,
                request.Length,
                out ignored,
                ref writeOverlapped);
            int error = Marshal.GetLastWin32Error();
            if (!completed && error != ErrorIoPending)
            {
                throw new Win32Exception(error, "Could not write Cloud III S HID request.");
            }

            if (WaitForSingleObject(writeOverlapped.EventHandle, 750) != WaitObject0)
            {
                CancelIoEx(handle, ref writeOverlapped);
                WaitForSingleObject(writeOverlapped.EventHandle, 250);
                throw new TimeoutException("Cloud III S HID write timed out.");
            }

            int written;
            if (!GetOverlappedResult(handle, ref writeOverlapped, out written, false) ||
                written != request.Length)
            {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(),
                    "Cloud III S HID request was not fully written.");
            }
        }
        finally
        {
            CloseHandle(writeOverlapped.EventHandle);
        }
    }

    private static List<HidInterface> EnumerateHyperxInterfaces()
    {
        List<HidInterface> interfaces = new List<HidInterface>();
        foreach (string path in EnumerateHidPaths())
        {
            using (SafeFileHandle handle = CreateFile(
                path,
                0,
                FileShareRead | FileShareWrite,
                IntPtr.Zero,
                OpenExisting,
                0,
                IntPtr.Zero))
            {
                if (handle.IsInvalid)
                {
                    continue;
                }

                HidAttributes attributes = new HidAttributes();
                attributes.Size = Marshal.SizeOf(typeof(HidAttributes));
                if (!HidD_GetAttributes(handle, ref attributes) ||
                    attributes.VendorId != HyperxVendorId ||
                    attributes.ProductId != CloudIIISProductId)
                {
                    continue;
                }

                HidpCaps caps;
                if (!TryGetCaps(handle, out caps))
                {
                    continue;
                }

                HidInterface item = new HidInterface();
                item.Path = path;
                item.Caps = caps;
                item.Product = ReadProduct(handle);
                interfaces.Add(item);
            }
        }
        return interfaces;
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

    private static string ReadProduct(SafeFileHandle handle)
    {
        IntPtr buffer = Marshal.AllocHGlobal(512);
        try
        {
            return HidD_GetProductString(handle, buffer, 512)
                ? Marshal.PtrToStringUni(buffer)
                : "";
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

    private static Overlapped CreateOverlapped()
    {
        Overlapped value = new Overlapped();
        value.EventHandle = CreateEvent(IntPtr.Zero, true, false, null);
        if (value.EventHandle == IntPtr.Zero)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not create I/O event.");
        }
        return value;
    }

    private static string Format(byte[] value)
    {
        return value == null ? "<none>" : BitConverter.ToString(value);
    }
}
