using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

internal static class HidppQuery
{
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareRead = 0x00000001;
    private const uint FileShareWrite = 0x00000002;
    private const uint OpenExisting = 3;
    private const uint FileFlagOverlapped = 0x40000000;
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

    public static int Main(string[] args)
    {
        if (args == null || args.Length < 1)
        {
            Console.Error.WriteLine("Pass the HID++ long-report device path.");
            return 2;
        }

        using (SafeFileHandle handle = CreateFile(
            args[0],
            GenericRead | GenericWrite,
            FileShareRead | FileShareWrite,
            IntPtr.Zero,
            OpenExisting,
            FileFlagOverlapped,
            IntPtr.Zero))
        {
            if (handle.IsInvalid)
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not open HID++ interface.");
            }

            const byte deviceIndex = 1;
            byte[] batteryFeature = Request(
                handle,
                deviceIndex,
                0x00,
                0x00,
                new byte[] { 0x10, 0x04 });
            Console.WriteLine("ROOT_1004=" + Format(batteryFeature));

            if (batteryFeature == null || batteryFeature.Length < 5 || batteryFeature[4] == 0)
            {
                return 3;
            }

            byte[] battery = Request(
                handle,
                deviceIndex,
                batteryFeature[4],
                0x10,
                new byte[0]);
            Console.WriteLine("BATTERY=" + Format(battery));
            if (battery != null && battery.Length >= 8)
            {
                Console.WriteLine(
                    "BATTERY_PERCENT={0} LEVEL_FLAGS={1:X2} STATUS={2:X2} EXTERNAL_POWER={3:X2}",
                    battery[4],
                    battery[5],
                    battery[6],
                    battery[7]);
            }

            byte[] nameFeature = Request(
                handle,
                deviceIndex,
                0x00,
                0x00,
                new byte[] { 0x00, 0x05 });
            Console.WriteLine("ROOT_0005=" + Format(nameFeature));
            if (nameFeature != null && nameFeature.Length >= 5 && nameFeature[4] != 0)
            {
                byte featureIndex = nameFeature[4];
                byte[] count = Request(handle, deviceIndex, featureIndex, 0x00, new byte[0]);
                Console.WriteLine("NAME_COUNT=" + Format(count));
                if (count != null && count.Length >= 5 && count[4] > 0 && count[4] <= 64)
                {
                    int nameLength = count[4];
                    List<byte> name = new List<byte>();
                    for (int offset = 0; offset < nameLength; offset += 16)
                    {
                        byte[] fragment = Request(
                            handle,
                            deviceIndex,
                            featureIndex,
                            0x10,
                            new byte[] { (byte)offset });
                        Console.WriteLine("NAME_FRAGMENT=" + Format(fragment));
                        if (fragment == null)
                        {
                            break;
                        }

                        int countToCopy = Math.Min(nameLength - offset, fragment.Length - 4);
                        for (int index = 0; index < countToCopy; index++)
                        {
                            name.Add(fragment[4 + index]);
                        }
                    }
                    Console.WriteLine("DEVICE_NAME=" + Encoding.UTF8.GetString(name.ToArray()).TrimEnd('\0'));
                }
            }
        }

        return 0;
    }

    private static byte[] Request(
        SafeFileHandle handle,
        byte deviceIndex,
        byte featureIndex,
        byte function,
        byte[] parameters)
    {
        byte[] request = new byte[20];
        request[0] = 0x11;
        request[1] = deviceIndex;
        request[2] = featureIndex;
        request[3] = (byte)(function | 0x08);
        if (parameters != null && parameters.Length > 0)
        {
            Buffer.BlockCopy(parameters, 0, request, 4, Math.Min(16, parameters.Length));
        }

        DateTime deadline = DateTime.UtcNow.AddMilliseconds(1000);
        while (DateTime.UtcNow < deadline)
        {
            byte[] response = new byte[20];
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
                    throw new Win32Exception(readError, "Could not start HID++ read.");
                }

                Write(handle, request);

                int remaining = Math.Max(1, (int)(deadline - DateTime.UtcNow).TotalMilliseconds);
                if (WaitForSingleObject(readOverlapped.EventHandle, (uint)remaining) != WaitObject0)
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

                if (readCount >= 7 &&
                    response[1] == deviceIndex &&
                    response[2] == featureIndex &&
                    response[3] == (byte)(function | 0x08))
                {
                    return response;
                }

                if (readCount >= 7 &&
                    response[1] == deviceIndex &&
                    response[2] == 0xFF &&
                    response[3] == featureIndex &&
                    response[4] == (byte)(function | 0x08))
                {
                    Console.WriteLine("HIDPP_ERROR=" + Format(response));
                    return null;
                }
            }
            finally
            {
                CloseHandle(readOverlapped.EventHandle);
            }
        }

        return null;
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
                throw new Win32Exception(error, "Could not write HID++ request.");
            }

            if (WaitForSingleObject(writeOverlapped.EventHandle, 500) != WaitObject0)
            {
                CancelIoEx(handle, ref writeOverlapped);
                WaitForSingleObject(writeOverlapped.EventHandle, 250);
                throw new TimeoutException("HID++ write timed out.");
            }

            int written;
            if (!GetOverlappedResult(handle, ref writeOverlapped, out written, false) ||
                written != request.Length)
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "HID++ request was not fully written.");
            }
        }
        finally
        {
            CloseHandle(writeOverlapped.EventHandle);
        }
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
