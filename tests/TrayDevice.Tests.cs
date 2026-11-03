using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.Serialization;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using CodexUsageTray;

internal static class TrayDeviceTests
{
    [STAThread]
    private static void Main()
    {
        AppSettings settings = new AppSettings();
        settings.IconMetric = AppSettings.IconMetricDevice;
        settings.IconDeviceId = "id-b";
        settings.IconDeviceName = "Mouse";
        JavaScriptSerializer serializer = new JavaScriptSerializer();
        AppSettings restored = serializer.Deserialize<AppSettings>(serializer.Serialize(settings));
        Assert(restored.IconMetric == AppSettings.IconMetricDevice && restored.IconDeviceId == "id-b",
            "device setting survives serialization");
        AppSettings copy = new AppSettings();
        copy.CopyValuesFrom(restored);
        Assert(copy.IconDeviceId == "id-b" && copy.IconDeviceName == "Mouse", "device setting is copied");
        TestLearnedRateSettings();

        UsageSnapshot snapshot = new UsageSnapshot();
        snapshot.LowestBatteryPercent = 20;
        snapshot.AverageBatteryPercent = 50;
        snapshot.Devices.Add(Device("id-a", "Mouse", 20));
        snapshot.Devices.Add(Device("id-b", "Mouse", 80));
        TrayAppContext context = (TrayAppContext)FormatterServices.GetUninitializedObject(typeof(TrayAppContext));
        typeof(TrayAppContext).GetField("settings", BindingFlags.Instance | BindingFlags.NonPublic)
            .SetValue(context, settings);
        MethodInfo getIcon = typeof(TrayAppContext).GetMethod("GetIconWindow", BindingFlags.Instance | BindingFlags.NonPublic);
        Assert(((LimitWindow)getIcon.Invoke(context, new object[] { snapshot })).BatteryPercent == 80,
            "selects by ID, not name or lowest value");
        snapshot.Devices[1].IsCached = true;
        Assert(((LimitWindow)getIcon.Invoke(context, new object[] { snapshot })).BatteryPercent == 80,
            "retains selected device's last reported value");
        snapshot.Devices.RemoveAt(1);
        Assert(getIcon.Invoke(context, new object[] { snapshot }) == null,
            "missing selected device does not fall back to another device");
        settings.IconMetric = AppSettings.IconMetricWeekly;
        Assert(((LimitWindow)getIcon.Invoke(context, new object[] { snapshot })).BatteryPercent == 20,
            "lowest mode still works");
        settings.IconMetric = AppSettings.IconMetricFiveHour;
        Assert(((LimitWindow)getIcon.Invoke(context, new object[] { snapshot })).BatteryPercent == 50,
            "average mode still works");

        settings.IconMetric = AppSettings.IconMetricDevice;
        using (SettingsForm form = new SettingsForm(settings, snapshot))
        {
            ComboBox combo = (ComboBox)typeof(SettingsForm).GetField("deviceCombo",
                BindingFlags.Instance | BindingFlags.NonPublic).GetValue(form);
            Assert(combo.SelectedItem != null && combo.SelectedItem.ToString().Contains("unavailable"),
                "missing saved selection remains visible");
            snapshot.Devices.Add(Device("id-b", "Mouse", 80));
            form.UpdateDevices(snapshot);
            Assert(combo.SelectedItem != null && !combo.SelectedItem.ToString().Contains("unavailable"),
                "saved selection reconnects without switching to the same-named device");
        }
        Console.WriteLine("Tray device selection tests passed.");
    }

    private static void TestLearnedRateSettings()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BluetoothRateSettings-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "settings.json");
        string backup = path + ".bak";
        BindingFlags flags = BindingFlags.NonPublic | BindingFlags.Static;
        MethodInfo load = typeof(AppSettings).GetMethod("LoadFromPath", flags);
        MethodInfo saveRates = typeof(AppSettings).GetMethod("TrySaveLearnedBatteryRatesToPath", BindingFlags.NonPublic | BindingFlags.Instance);
        List<LearnedBatteryRate> rates = new List<LearnedBatteryRate>
        {
            new LearnedBatteryRate { DeviceId = "mouse", DischargeSamples = new List<LearnedBatteryRateSample>
                { new LearnedBatteryRateSample { Percent = 3, Seconds = 3600 } } }
        };
        try
        {
            AppSettings settings = (AppSettings)load.Invoke(null, new object[] { path });
            settings.RefreshSeconds = 120;
            Assert((bool)saveRates.Invoke(settings, new object[] { path, rates }), "learned rates save to a new settings file");
            AppSettings restored = (AppSettings)load.Invoke(null, new object[] { path });
            Assert(restored.RefreshSeconds == 120 && restored.LearnedBatteryRates.Count == 1 &&
                restored.LearnedBatteryRates[0].DischargeSamples[0].Seconds == 3600,
                "learned rates and existing settings survive a restart");
            BatteryRateTracker restartedTracker = new BatteryRateTracker();
            restartedTracker.ImportHistory(restored.LearnedBatteryRates);
            BatteryProbeResult restartedMouse = new BatteryProbeResult { Id = "mouse", Percent = 50, Status = "Discharging" };
            restartedTracker.Update(new List<BatteryProbeResult> { restartedMouse }, 1);
            Assert(restartedMouse.BatteryEstimate != null && restartedMouse.BatteryEstimate.StartsWith("Avg.", StringComparison.Ordinal),
                "a saved average is available immediately after reloading settings");
            AppSettings clone = restored.Clone();
            clone.LearnedBatteryRates[0].DischargeSamples[0].Percent = 9;
            Assert(restored.LearnedBatteryRates[0].DischargeSamples[0].Percent == 3,
                "settings copies do not share mutable rate history");

            string valid = File.ReadAllText(path);
            JavaScriptSerializer serializer = new JavaScriptSerializer();
            IDictionary<string, object> values = (IDictionary<string, object>)serializer.DeserializeObject(valid);
            values["RefreshSeconds"] = 180;
            string externalEdit = serializer.Serialize(values);
            File.WriteAllText(path, externalEdit);
            Assert(!(bool)saveRates.Invoke(restored, new object[] { path, rates }) && File.ReadAllText(path) == externalEdit,
                "automatic rate saves do not overwrite an externally edited settings file");

            File.WriteAllText(path, "{broken");
            File.WriteAllText(backup, valid);
            AppSettings recovered = (AppSettings)load.Invoke(null, new object[] { path });
            Assert(recovered.RefreshSeconds == 120 && !(bool)saveRates.Invoke(recovered, new object[] { path, rates }) &&
                File.ReadAllText(path) == "{broken" && File.ReadAllText(backup) == valid,
                "automatic rate saves preserve invalid primary files and recovery backups");

            string malformedRates = "{\"SettingsVersion\":" + AppSettings.CurrentSettingsVersion +
                ",\"RefreshSeconds\":120,\"LearnedBatteryRates\":\"invalid\"}";
            File.WriteAllText(path, malformedRates);
            File.Delete(backup);
            AppSettings partial = (AppSettings)load.Invoke(null, new object[] { path });
            Assert(partial.RefreshSeconds == 120 && partial.LearnedBatteryRates.Count == 0 &&
                !(bool)saveRates.Invoke(partial, new object[] { path, rates }) && File.ReadAllText(path) == malformedRates,
                "malformed rate data does not erase other settings or trigger an automatic overwrite");

            string newer = "{\"SettingsVersion\":" + (AppSettings.CurrentSettingsVersion + 1) + ",\"RefreshSeconds\":180}";
            File.WriteAllText(path, newer);
            AppSettings protectedSettings = (AppSettings)load.Invoke(null, new object[] { path });
            Assert(!(bool)saveRates.Invoke(protectedSettings, new object[] { path, rates }) && File.ReadAllText(path) == newer,
                "automatic rate saves respect newer settings versions");
        }
        finally
        {
            if (File.Exists(path)) File.Delete(path);
            if (File.Exists(backup)) File.Delete(backup);
            Directory.Delete(directory);
        }
    }

    private static LimitWindow Device(string id, string name, int percent)
    {
        return new LimitWindow { DeviceId = id, DisplayName = name, DetailLeft = "Wireless", BatteryPercent = percent,
            UsedPercent = 100 - percent };
    }

    private static void Assert(bool condition, string name)
    {
        if (!condition) throw new Exception(name);
    }
}
