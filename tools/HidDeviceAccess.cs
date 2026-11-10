using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace CodexUsageTray
{
    internal sealed class HidDeviceInfo
    {
        public string Path;
        public string ProductName;
        public string ManufacturerName;
        public ushort VendorId;
        public ushort ProductId;
        public ushort UsagePage;
        public ushort Usage;
        public int InputReportLength;
        public int OutputReportLength;
        public int FeatureReportLength;
    }

    internal sealed class HidDeviceConnection : IDisposable
    {
        private const uint GenericRead = 0x80000000;
        private const uint GenericWrite = 0x40000000;
        private const uint FileShareRead = 0x00000001;
        private const uint FileShareWrite = 0x00000002;
        private const uint OpenExisting = 3;
        private const uint FileFlagOverlapped = 0x40000000;
        private const int ErrorInvalidFunction = 1;
        private const int ErrorIoPending = 997;
        private const uint WaitObject0 = 0;

        [StructLayout(LayoutKind.Sequential)]
        private struct Overlapped
        {
            public IntPtr Internal;
            public IntPtr InternalHigh;
            public uint Offset;
            public uint OffsetHigh;
            public IntPtr EventHandle;
        }

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
        private static extern bool HidD_SetFeature(
            SafeFileHandle hidDeviceObject,
            byte[] reportBuffer,
            int reportBufferLength);

        private readonly HidDeviceInfo deviceInfo;
        private readonly SafeFileHandle handle;
        private bool disposed;

        public HidDeviceConnection(HidDeviceInfo deviceInfo)
        {
            if (deviceInfo == null)
            {
                throw new ArgumentNullException("deviceInfo");
            }

            this.deviceInfo = deviceInfo;
            handle = CreateFile(
                deviceInfo.Path,
                GenericRead | GenericWrite,
                FileShareRead | FileShareWrite,
                IntPtr.Zero,
                OpenExisting,
                FileFlagOverlapped,
                IntPtr.Zero);
            if (handle.IsInvalid)
            {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(),
                    "Could not open HID interface.");
            }
        }

        public byte[] Exchange(
            byte[] request,
            Func<byte[], bool> responseMatcher,
            int timeoutMilliseconds,
            bool allowFeatureReportFallback)
        {
            ThrowIfDisposed();
            if (request == null || request.Length == 0)
            {
                throw new ArgumentException("A HID request is required.", "request");
            }

            DateTime deadline = DateTime.UtcNow.AddMilliseconds(
                Math.Max(1, timeoutMilliseconds));
            bool requestWritten = false;

            while (DateTime.UtcNow < deadline)
            {
                int inputLength = Math.Max(1, deviceInfo.InputReportLength);
                byte[] response = new byte[inputLength];
                Overlapped readOverlapped = CreateOverlapped();
                bool readPending = false;
                try
                {
                    int immediateReadCount;
                    bool readCompleted = ReadFile(
                        handle,
                        response,
                        response.Length,
                        out immediateReadCount,
                        ref readOverlapped);
                    if (!readCompleted)
                    {
                        int readError = Marshal.GetLastWin32Error();
                        if (readError != ErrorIoPending)
                        {
                            throw new Win32Exception(readError, "Could not start HID read.");
                        }
                        readPending = true;
                    }

                    if (!requestWritten)
                    {
                        WriteRequest(request, allowFeatureReportFallback);
                        requestWritten = true;
                    }

                    int readCount;
                    if (readCompleted)
                    {
                        readCount = immediateReadCount;
                    }
                    else
                    {
                        int remaining = Math.Max(
                            1,
                            (int)(deadline - DateTime.UtcNow).TotalMilliseconds);
                        if (WaitForSingleObject(
                            readOverlapped.EventHandle,
                            (uint)remaining) != WaitObject0)
                        {
                            CancelAndDrain(ref readOverlapped);
                            readPending = false;
                            return null;
                        }

                        if (!GetOverlappedResult(
                            handle,
                            ref readOverlapped,
                            out readCount,
                            false))
                        {
                            readPending = false;
                            return null;
                        }
                        readPending = false;
                    }

                    if (readCount <= 0)
                    {
                        continue;
                    }
                    if (readCount != response.Length)
                    {
                        Array.Resize(ref response, readCount);
                    }
                    if (responseMatcher == null || responseMatcher(response))
                    {
                        return response;
                    }
                }
                finally
                {
                    if (readPending)
                    {
                        CancelAndDrain(ref readOverlapped);
                    }
                    CloseHandle(readOverlapped.EventHandle);
                }
            }

            return null;
        }

        private void WriteRequest(byte[] request, bool allowFeatureReportFallback)
        {
            if (deviceInfo.OutputReportLength <= 0 && deviceInfo.FeatureReportLength > 0)
            {
                SetFeatureReport(request);
                return;
            }

            try
            {
                WriteOutputReport(request);
            }
            catch (Win32Exception ex)
            {
                if (!allowFeatureReportFallback ||
                    ex.NativeErrorCode != ErrorInvalidFunction ||
                    deviceInfo.FeatureReportLength <= 0)
                {
                    throw;
                }
                SetFeatureReport(request);
            }
        }

        private void WriteOutputReport(byte[] request)
        {
            byte[] report = ResizeReport(
                request,
                deviceInfo.OutputReportLength > 0
                    ? deviceInfo.OutputReportLength
                    : request.Length);
            Overlapped writeOverlapped = CreateOverlapped();
            bool writePending = false;
            try
            {
                int immediateWriteCount;
                bool writeCompleted = WriteFile(
                    handle,
                    report,
                    report.Length,
                    out immediateWriteCount,
                    ref writeOverlapped);
                if (!writeCompleted)
                {
                    int writeError = Marshal.GetLastWin32Error();
                    if (writeError != ErrorIoPending)
                    {
                        throw new Win32Exception(writeError, "Could not write HID report.");
                    }
                    writePending = true;
                }

                int written;
                if (writeCompleted)
                {
                    written = immediateWriteCount;
                }
                else
                {
                    if (WaitForSingleObject(writeOverlapped.EventHandle, 750) != WaitObject0)
                    {
                        CancelAndDrain(ref writeOverlapped);
                        writePending = false;
                        throw new TimeoutException("HID write timed out.");
                    }
                    if (!GetOverlappedResult(
                        handle,
                        ref writeOverlapped,
                        out written,
                        false))
                    {
                        writePending = false;
                        throw new Win32Exception(
                            Marshal.GetLastWin32Error(),
                            "Could not complete HID write.");
                    }
                    writePending = false;
                }

                if (written != report.Length)
                {
                    throw new IOException(
                        "The HID report was not fully written.");
                }
            }
            finally
            {
                if (writePending)
                {
                    CancelAndDrain(ref writeOverlapped);
                }
                CloseHandle(writeOverlapped.EventHandle);
            }
        }

        private void SetFeatureReport(byte[] request)
        {
            byte[] report = ResizeReport(
                request,
                deviceInfo.FeatureReportLength > 0
                    ? deviceInfo.FeatureReportLength
                    : request.Length);
            if (!HidD_SetFeature(handle, report, report.Length))
            {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(),
                    "Could not write HID feature report.");
            }
        }

        private void CancelAndDrain(ref Overlapped overlapped)
        {
            CancelIoEx(handle, ref overlapped);
            WaitForSingleObject(overlapped.EventHandle, 250);
        }

        private static byte[] ResizeReport(byte[] source, int length)
        {
            byte[] report = new byte[Math.Max(1, length)];
            Buffer.BlockCopy(source, 0, report, 0, Math.Min(source.Length, report.Length));
            return report;
        }

        private static Overlapped CreateOverlapped()
        {
            Overlapped value = new Overlapped();
            value.EventHandle = CreateEvent(IntPtr.Zero, true, false, null);
            if (value.EventHandle == IntPtr.Zero)
            {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(),
                    "Could not create HID I/O event.");
            }
            return value;
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException("HidDeviceConnection");
            }
        }

        public void Dispose()
        {
            if (disposed)
            {
                return;
            }
            disposed = true;
            handle.Dispose();
        }
    }

    internal static class HidDeviceEnumerator
    {
        private const uint DigcfPresent = 0x00000002;
        private const uint DigcfDeviceInterface = 0x00000010;
        private const uint FileShareRead = 0x00000001;
        private const uint FileShareWrite = 0x00000002;
        private const uint OpenExisting = 3;

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
        private static extern bool HidD_GetProductString(
            SafeFileHandle hidDeviceObject,
            IntPtr buffer,
            int bufferLength);

        [DllImport("hid.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool HidD_GetManufacturerString(
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
        private static extern int HidP_GetCaps(
            IntPtr preparsedData,
            out HidpCaps capabilities);

        public static List<HidDeviceInfo> Enumerate()
        {
            List<HidDeviceInfo> devices = new List<HidDeviceInfo>();
            foreach (string path in EnumeratePaths())
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
                    HidpCaps caps;
                    if (!HidD_GetAttributes(handle, ref attributes) ||
                        !TryGetCaps(handle, out caps))
                    {
                        continue;
                    }

                    HidDeviceInfo device = new HidDeviceInfo();
                    device.Path = path;
                    device.ProductName = ReadString(handle, HidD_GetProductString);
                    device.ManufacturerName = ReadString(handle, HidD_GetManufacturerString);
                    device.VendorId = attributes.VendorId;
                    device.ProductId = attributes.ProductId;
                    device.UsagePage = caps.UsagePage;
                    device.Usage = caps.Usage;
                    device.InputReportLength = caps.InputReportByteLength;
                    device.OutputReportLength = caps.OutputReportByteLength;
                    device.FeatureReportLength = caps.FeatureReportByteLength;
                    devices.Add(device);
                }
            }

            return devices;
        }

        private delegate bool HidStringReader(
            SafeFileHandle hidDeviceObject,
            IntPtr buffer,
            int bufferLength);

        private static string ReadString(
            SafeFileHandle handle,
            HidStringReader reader)
        {
            IntPtr buffer = Marshal.AllocHGlobal(512);
            try
            {
                return reader(handle, buffer, 512)
                    ? Marshal.PtrToStringUni(buffer)
                    : "";
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
        }

        private static bool TryGetCaps(
            SafeFileHandle handle,
            out HidpCaps caps)
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

        private static List<string> EnumeratePaths()
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
                            string path = Marshal.PtrToStringAuto(
                                IntPtr.Add(detailData, 4));
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
}
