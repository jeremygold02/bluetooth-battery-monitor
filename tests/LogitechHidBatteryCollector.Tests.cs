using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Text;
using CodexUsageTray;

internal static class LogitechHidBatteryCollectorTests
{
    private static void Main()
    {
        TestInterfaceSelection();
        TestX3SparseSlotsAndFallback();
        TestX3UnsupportedUnifiedFeatureFallsBack();
        TestOptionalNameErrorsPreserveValidBattery();
        TestNameErrorsWithoutBatteryAndUnexpectedErrorsPropagate();
        TestX3NoResponseHasNoGhost();
        TestResultsSurviveLaterInterfaceError();
        TestLegacyReceiverStillStopsAtFirstTimeout();
        TestLegacyReceiverSuccessAndFeatureOrder();
        Console.WriteLine("Logitech HID++ collector tests passed.");
    }

    private static void TestInterfaceSelection()
    {
        HidDeviceInfo x3 = Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20);
        Assert(LogitechHidBatteryCollector.IsSupportedInterface(x3), "X3 receiver long interface must be accepted");
        Assert(!LogitechHidBatteryCollector.IsSupportedInterface(Device(0x046D, 0xC54F, 0xFF43, 0x0301, 7, 7)), "short collection must not be queried as long HID++");
        Assert(!LogitechHidBatteryCollector.IsSupportedInterface(Device(0x046D, 0xC54E, 0xFF43, 0x0302, 20, 20)), "unknown FF43 product must not be queried");
        Assert(!LogitechHidBatteryCollector.IsSupportedInterface(Device(0x1234, 0xC54F, 0xFF43, 0x0302, 20, 20)), "other vendors must not be queried");
        Assert(!LogitechHidBatteryCollector.IsSupportedInterface(Device(0x046D, 0xC54F, 0xFF43, 0x0302, 19, 20)), "short input must be rejected");
        Assert(!LogitechHidBatteryCollector.IsSupportedInterface(Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 19)), "short output must be rejected");
        Assert(LogitechHidBatteryCollector.IsSupportedInterface(Device(0x046D, 0xC53F, 0xFF00, 0x0002, 20, 20)), "existing Logitech HID++ interface must remain supported");
        Assert(!LogitechHidBatteryCollector.IsSupportedInterface(null), "null interface must be ignored");
    }

    private static void TestX3SparseSlotsAndFallback()
    {
        HidDeviceInfo device = Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20);
        List<byte[]> requests = new List<byte[]>();
        LogitechHidBatteryCollector.HidExchange exchange = delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback)
        {
            requests.Add((byte[])request.Clone());
            Assert(request.Length == 20 && request[0] == 0x11 && timeout == 650 && !fallback, "unexpected HID++ request format");
            Assert(request[1] >= 1 && request[1] <= 6, "receiver address must not be treated as a mouse");
            byte slot = request[1];
            if (slot == 2 || slot == 3 || slot == 5) return null;
            if (slot == 6) return ErrorReply(request, match);
            if (request[2] == 0)
            {
                ushort feature = (ushort)((request[4] << 8) | request[5]);
                if (feature == 0x1004) return Reply(request, match, 4, 0, 0);
                if (feature == 0x1000 && slot == 4) return Reply(request, match, 5, 0, 0);
                if (feature == 0x0005) return Reply(request, match, 0, 0, 0);
                throw new Exception("unexpected root feature");
            }
            if (request[2] == 4 && request[3] == 0x18)
                return slot == 4 ? null : Reply(request, match, 72, 0, 1);
            if (request[2] == 5 && request[3] == 0x08)
                return Reply(request, match, 37, 0, 0);
            throw new Exception("unexpected feature request");
        };

        List<BatteryProbeResult> found = ReadTestDevices(device, exchange);
        Assert(found.Count == 2, "live devices in sparse slots must both be retained");
        Assert(found[0].Id.EndsWith(":1") && found[0].Percent == 72 && found[0].Status == "Charging", "unified battery response was not parsed");
        Assert(found[1].Id.EndsWith(":4") && found[1].Percent == 37 && found[1].Status == "Discharging", "legacy battery fallback was not parsed");
        Assert(found[0].Name == "Logitech wireless device", "missing device name must use the normal fallback");
        for (byte slot = 1; slot <= 6; slot++)
        {
            List<byte[]> slotRequests = requests.FindAll(delegate(byte[] item) { return item[1] == slot; });
            Assert(slotRequests.Count > 0, "all receiver slots must be checked");
            Assert(slotRequests[0][2] == 0 && slotRequests[0][4] == 0x10 && slotRequests[0][5] == 0x04, "each slot must try unified battery first");
            if (slot == 2 || slot == 3 || slot == 5 || slot == 6)
                Assert(slotRequests.Count == 1, "a silent or rejected slot must use only one request");
        }
    }

    private static void TestX3NoResponseHasNoGhost()
    {
        int requests = 0;
        List<BatteryProbeResult> found = ReadTestDevices(
            Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20),
            delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback) { requests++; return null; });
        Assert(found.Count == 0 && requests == 6, "an unresponsive receiver must not create a phantom mouse or retry empty slots");
    }

    private static void TestX3UnsupportedUnifiedFeatureFallsBack()
    {
        List<byte[]> requests = new List<byte[]>();
        List<BatteryProbeResult> found = ReadTestDevices(
            Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20),
            delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback)
            {
                requests.Add((byte[])request.Clone());
                if (request[1] != 1) return null;
                if (request[2] == 0 && request[4] == 0x10 && request[5] == 0x04) return Reply(request, match, 0, 0, 0);
                if (request[2] == 0 && request[4] == 0x10 && request[5] == 0x00) return Reply(request, match, 5, 0, 0);
                if (request[2] == 5) return Reply(request, match, 55, 0, 0);
                if (request[2] == 0 && request[4] == 0 && request[5] == 5) return Reply(request, match, 0, 0, 0);
                throw new Exception("unexpected fallback request");
            });
        Assert(found.Count == 1 && found[0].Percent == 55, "unsupported unified feature must fall back to battery status");
        Assert(requests[0][4] == 0x10 && requests[0][5] == 0x04 && requests[1][4] == 0x10 && requests[1][5] == 0,
            "fallback must try the confirmed unified feature before battery status");
    }

    private static void TestResultsSurviveLaterInterfaceError()
    {
        List<BatteryProbeResult> found = new List<BatteryProbeResult>();
        bool failed = false;
        try
        {
            LogitechHidBatteryCollector.ReadDevices(Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20),
                delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback)
                {
                    if (request[1] == 2) throw new InvalidOperationException("device disconnected");
                    if (request[2] == 0 && request[4] == 0x10 && request[5] == 0x04) return Reply(request, match, 4, 0, 0);
                    if (request[2] == 4) return Reply(request, match, 61, 0, 0);
                    if (request[2] == 0 && request[4] == 0 && request[5] == 5) return Reply(request, match, 0, 0, 0);
                    throw new Exception("unexpected request");
                }, found);
        }
        catch (InvalidOperationException) { failed = true; }
        Assert(failed && found.Count == 1 && found[0].Percent == 61, "a later interface failure must not discard an earlier device");
    }

    private static void TestOptionalNameErrorsPreserveValidBattery()
    {
        Exception[] errors = { new IOException("read failed"), new Win32Exception(5), new TimeoutException("write timed out") };
        foreach (Exception error in errors)
        {
            List<BatteryProbeResult> found = ReadTestDevices(Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20),
                delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback)
                {
                    if (request[1] != 1) return null;
                    if (request[2] == 0 && request[4] == 0x10 && request[5] == 0x04) return Reply(request, match, 4, 0, 0);
                    if (request[2] == 4) return Reply(request, match, 61, 0, 1);
                    if (request[2] == 0 && request[4] == 0 && request[5] == 5) throw error;
                    throw new Exception("unexpected request");
                });
            Assert(found.Count == 1 && found[0].Percent == 61 && found[0].Status == "Charging" &&
                found[0].Name == "Logitech wireless device", "an optional name I/O failure must preserve the battery reading");
        }
    }

    private static void TestNameErrorsWithoutBatteryAndUnexpectedErrorsPropagate()
    {
        foreach (bool validBattery in new bool[] { false, true })
        {
            bool threw = false;
            try
            {
                ReadTestDevices(Device(0x046D, 0xC54F, 0xFF43, 0x0302, 20, 20),
                    delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback)
                    {
                        if (request[1] != 1) return null;
                        if (request[2] == 0 && request[4] == 0x10 && request[5] == 0x04) return Reply(request, match, validBattery ? (byte)4 : (byte)0, 0, 0);
                        if (request[2] == 4) return Reply(request, match, 61, 0, 0);
                        if (request[2] == 0 && request[4] == 0x10 && request[5] == 0) return Reply(request, match, 0, 0, 0);
                        if (request[2] == 0 && request[4] == 0 && request[5] == 5)
                        {
                            if (validBattery) throw new InvalidOperationException("unexpected failure");
                            throw new IOException("read failed");
                        }
                        throw new Exception("unexpected request");
                    });
            }
            catch (IOException) { threw = !validBattery; }
            catch (InvalidOperationException) { threw = validBattery; }
            Assert(threw, "only expected optional-name I/O failures after a valid battery response may be ignored");
        }
    }

    private static void TestLegacyReceiverStillStopsAtFirstTimeout()
    {
        int requests = 0;
        List<BatteryProbeResult> found = ReadTestDevices(
            Device(0x046D, 0xC53F, 0xFF00, 0x0002, 20, 20),
            delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback) { requests++; return null; });
        Assert(found.Count == 1 && found[0].IsCached == false && found[0].Percent == null && requests == 1,
            "legacy receiver fallback and early exit must be preserved");
    }

    private static void TestLegacyReceiverSuccessAndFeatureOrder()
    {
        VerifyLegacyResult(true);
        VerifyLegacyResult(false);
    }

    private static void VerifyLegacyResult(bool primarySupported)
    {
        List<ushort> rootFeatures = new List<ushort>();
        List<BatteryProbeResult> found = ReadTestDevices(
            Device(0x046D, 0xC53F, 0xFF00, 0x0002, 20, 20),
            delegate(byte[] request, Func<byte[], bool> match, int timeout, bool fallback)
            {
                if (request[1] == 2) return null;
                Assert(request[1] == 1 && request[0] == 0x11 && request.Length == 20, "legacy HID++ addressing changed");
                if (request[2] == 0)
                {
                    ushort feature = (ushort)((request[4] << 8) | request[5]);
                    rootFeatures.Add(feature);
                    if (feature == 0x1000) return Reply(request, match, primarySupported ? (byte)3 : (byte)0, 0, 0);
                    if (feature == 0x1004) return Reply(request, match, 6, 0, 0);
                    if (feature == 0x0005) return Reply(request, match, primarySupported ? (byte)4 : (byte)0, 0, 0);
                    throw new Exception("unexpected legacy root feature");
                }
                if (request[2] == 3 && request[3] == 0x08) return Reply(request, match, 61, 0, 1);
                if (request[2] == 6 && request[3] == 0x18) return Reply(request, match, 86, 0, 0);
                if (request[2] == 4 && request[3] == 0x08) return Reply(request, match, 5, 0, 0);
                if (request[2] == 4 && request[3] == 0x18)
                {
                    byte[] response = new byte[20];
                    response[0] = 0x11; response[1] = request[1]; response[2] = request[2]; response[3] = request[3];
                    Buffer.BlockCopy(Encoding.UTF8.GetBytes("Mouse"), 0, response, 4, 5);
                    Assert(match(response), "legacy device name response must match");
                    return response;
                }
                throw new Exception("unexpected legacy feature request");
            });
        Assert(found.Count == 1, "legacy receiver must return exactly one device");
        Assert(found[0].Percent == (primarySupported ? 61 : 86) && found[0].Status == (primarySupported ? "Charging" : "Discharging"),
            "legacy receiver battery percentage or status changed");
        Assert(found[0].Name == (primarySupported ? "Mouse" : "Logitech wireless device"), "legacy receiver name handling changed");
        Assert(rootFeatures.Count == (primarySupported ? 2 : 3) && rootFeatures[0] == 0x1000 &&
            rootFeatures[rootFeatures.Count - 1] == 0x0005 && (primarySupported || rootFeatures[1] == 0x1004),
            "legacy receiver feature order changed");
    }

    private static HidDeviceInfo Device(ushort vendor, ushort product, ushort page, ushort usage, int input, int output)
    {
        return new HidDeviceInfo { Path = "fixture-path", VendorId = vendor, ProductId = product,
            UsagePage = page, Usage = usage, InputReportLength = input, OutputReportLength = output };
    }

    private static List<BatteryProbeResult> ReadTestDevices(HidDeviceInfo device, LogitechHidBatteryCollector.HidExchange exchange)
    {
        List<BatteryProbeResult> results = new List<BatteryProbeResult>();
        LogitechHidBatteryCollector.ReadDevices(device, exchange, results);
        return results;
    }

    private static byte[] Reply(byte[] request, Func<byte[], bool> match, byte value, byte level, byte status)
    {
        byte[] response = new byte[20];
        response[0] = 0x11; response[1] = request[1]; response[2] = request[2]; response[3] = request[3];
        response[4] = value; response[5] = level; response[6] = status;
        Assert(match(response), "valid response must match its request");
        return response;
    }

    private static byte[] ErrorReply(byte[] request, Func<byte[], bool> match)
    {
        byte[] response = new byte[20];
        response[0] = 0x11; response[1] = request[1]; response[2] = 0xFF;
        response[3] = request[2]; response[4] = request[3]; response[5] = 0x07;
        Assert(match(response), "protocol errors must be matched to their request");
        return response;
    }

    private static void Assert(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }
}
