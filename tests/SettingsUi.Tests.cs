using System;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using CodexUsageTray;

internal static class SettingsUiTests
{
    private const BindingFlags PrivateInstance = BindingFlags.NonPublic | BindingFlags.Instance;
    private const BindingFlags PrivateStatic = BindingFlags.NonPublic | BindingFlags.Static;

    [STAThread]
    private static void Main(string[] args)
    {
        string outputDirectory = args.Length > 0 ? Path.GetFullPath(args[0]) : null;
        if (outputDirectory != null) Directory.CreateDirectory(outputDirectory);
        AppSettings settings = new AppSettings();
        settings.Theme = AppSettings.ThemeDark;
        settings.IconMetric = AppSettings.IconMetricDevice;
        settings.IconDeviceId = "headset";
        settings.IconDeviceName = "HyperX Cloud III S Wireless";
        UsageSnapshot snapshot = new UsageSnapshot();
        snapshot.Devices.Add(new LimitWindow { DeviceId = "headset", DisplayName = "HyperX Cloud III S Wireless", BatteryPercent = 47, DetailLeft = "HyperX 2.4 GHz" });
        snapshot.Devices.Add(new LimitWindow { DeviceId = "device-a", DisplayName = "Mouse", BatteryPercent = 25 });
        snapshot.Devices.Add(new LimitWindow { DeviceId = "device-b", DisplayName = "Mouse", BatteryPercent = 55 });

        using (SettingsForm form = new SettingsForm(settings, snapshot))
        {
            FrameworkElement view = (FrameworkElement)Field(form, "settingsView");
            System.Windows.Forms.Integration.ElementHost elementHost =
                (System.Windows.Forms.Integration.ElementHost)form.Controls.Find("settingsWpfHost", true)[0];
            elementHost.Child = null;
            HwndSourceParameters parameters = new HwndSourceParameters("Headless settings checks")
            {
                Width = 960, Height = 640, PositionX = -30000, PositionY = -30000, WindowStyle = 0
            };
            using (HwndSource source = new HwndSource(parameters))
            {
                source.RootVisual = view;
                Layout(view, 960, 640);
                Check(!(bool)form.Visible, "settings form is never shown");
                Check((int)Field(view, "selectedPage") == 2, "tray icon page is initially selected");
                TestUnsavedChanges(form, view, snapshot);

                for (int page = 0; page < 4; page++)
                {
                    ShowPage(view, page);
                    Layout(view, 960, 640);
                    Render(view, 960, 640, outputDirectory, "settings-page-" + page + ".png");
                    StackPanel[] pages = (StackPanel[])Field(view, "pages");
                    for (int index = 0; index < pages.Length; index++)
                        Check(pages[index].Visibility == (index == page ? Visibility.Visible : Visibility.Collapsed), "only the selected page is visible");
                }

                ShowPage(view, 2);
                Layout(view, 960, 640);
                ScrollViewer mainScroll = FindByName<ScrollViewer>(view, "Settings content");
                Check(mainScroll.ExtentHeight <= mainScroll.ViewportHeight + 0.1, "default tray page fits without scrolling");
                MethodInfo applyTheme = view.GetType().GetMethod("ApplyViewTheme", PrivateInstance);
                applyTheme.Invoke(view, new object[] { true, false });
                Layout(view, 960, 640);
                Wpf.Ui.Controls.Button[] navigation = (Wpf.Ui.Controls.Button[])Field(view, "navigation");
                Check(navigation[2].Background == SystemColors.HighlightBrush && navigation[2].Foreground == SystemColors.HighlightTextBrush,
                    "high contrast uses system selection colors");
                Wpf.Ui.Controls.Button save = (Wpf.Ui.Controls.Button)Field(view, "saveAction");
                Check(save.Background == SystemColors.HighlightBrush && save.Foreground == SystemColors.HighlightTextBrush,
                    "high contrast save button uses system colors");
                applyTheme.Invoke(view, new object[] { false, true });
                Layout(view, 960, 640);
                TextBlock label = FindText(view, "Background color");
                Check(label != null && label.ActualWidth >= TextWidth(label), "background color label is not clipped");
                ComboBox devices = (ComboBox)Field(view, "device");
                Check(devices.Items.Count == 3 && devices.SelectedIndex == 0 && devices.IsEnabled,
                    "the selected headset is available in the device selector");
                RadioButton lowest = (RadioButton)Field(view, "lowest");
                RadioButton average = (RadioButton)Field(view, "average");
                RadioButton specific = (RadioButton)Field(view, "specific");
                lowest.IsChecked = true;
                Check(!devices.IsEnabled, "lowest battery disables the device selector");
                average.IsChecked = true;
                Check(!devices.IsEnabled, "average battery disables the device selector");
                specific.IsChecked = true;
                Check(devices.IsEnabled, "specific device enables the device selector");
                devices.SelectedIndex = 1;
                devices.SelectedIndex = 2;
                Wpf.Ui.Controls.Button auto = (Wpf.Ui.Controls.Button)Field(view, "autoColor");
                Check(auto.IsEnabled && AutomationProperties.GetHelpText(auto).StartsWith("Selected"), "automatic background shows its selected state");
                Click(FindByName<Wpf.Ui.Controls.Button>(view, "Dark teal background"));
                Click(FindByName<Wpf.Ui.Controls.Button>(view, "Black text"));
                Check(AutomationProperties.GetHelpText(FindByName<Wpf.Ui.Controls.Button>(view, "Black text")) == "Selected",
                    "selected text color is exposed to assistive technology");
                Check(AutomationProperties.GetHelpText(auto) == "Use automatic background color", "automatic background is available after choosing a preset");
                BitmapSource iconPreview = (BitmapSource)((Image)Field(view, "preview")).Source;
                Check(iconPreview.PixelWidth == 36 && iconPreview.PixelHeight == 36, "preview is rendered crisply at its displayed size");
                byte[] pixel = FirstPixel(iconPreview);
                Check(pixel[0] == 83 && pixel[1] == 88 && pixel[2] == 0 && pixel[3] == 255, "preview uses the chosen background color");
                CheckBox background = (CheckBox)Field(view, "background");
                background.IsChecked = false;
                iconPreview = (BitmapSource)((Image)Field(view, "preview")).Source;
                Check(FirstPixel(iconPreview)[3] == 0, "preview background becomes transparent when disabled");
                background.IsChecked = true;

                Wpf.Ui.Controls.NumberBox preview = (Wpf.Ui.Controls.NumberBox)Field(view, "previewPercent");
                preview.Value = 100;
                Layout(view, 960, 640);
                Check(preview.Text == "100", "preview number displays a three-digit value");
                AssertNumberInputFits(preview);
                ComboBox theme = (ComboBox)Field(view, "theme");
                theme.SelectedItem = AppSettings.ThemeLight;
                Layout(view, 960, 640);
                Check(settings.Theme == AppSettings.ThemeDark,
                    "changing theme controls does not modify settings before save");

                ShowPage(view, 0);
                int updateRequests = 0;
                form.UpdateCheckRequested += delegate { updateRequests++; };
                Wpf.Ui.Controls.Button checkUpdates = FindByName<Wpf.Ui.Controls.Button>(view, "Check for updates");
                Check(checkUpdates != null && checkUpdates.IsEnabled, "update check is available and accessible");
                Click(checkUpdates);
                Check(updateRequests == 1, "update check raises one request");
                form.SetUpdateButtonState(true, "Checking...");
                form.SetUpdateStatus("Checking for updates...");
                Check(!checkUpdates.IsEnabled && (string)checkUpdates.Content == "Checking...", "busy update check disables the action");
                Click(checkUpdates);
                Check(updateRequests == 1, "disabled update action does not raise a request");
                Check(((TextBlock)Field(view, "updateStatus")).Text == "Checking for updates...", "update status is visible");
                form.SetUpdateButtonState(false, null);
                Check(checkUpdates.IsEnabled && (string)checkUpdates.Content == "Check for updates", "update action is restored after checking");
                Wpf.Ui.Controls.NumberBox refresh = (Wpf.Ui.Controls.NumberBox)Field(view, "refresh");
                refresh.Value = 120;
                Layout(view, 960, 640);
                Check(refresh.Text == "120", "polling value is visible");
                AssertNumberInputFits(refresh);
                ShowPage(view, 1);
                Wpf.Ui.Controls.NumberBox critical = (Wpf.Ui.Controls.NumberBox)Field(view, "critical");
                Wpf.Ui.Controls.NumberBox low = (Wpf.Ui.Controls.NumberBox)Field(view, "low");
                critical.Value = 35;
                Check(low.Value == 35, "raising critical adjusts low threshold");
                low.Value = 20;
                Check(critical.Value == 20, "lowering low adjusts critical threshold");

                AppSettings candidate = (AppSettings)typeof(SettingsForm).GetMethod("CreateSettingsCandidate", PrivateInstance).Invoke(form, null);
                Check(candidate.RefreshSeconds == 120 && candidate.IconMetric == AppSettings.IconMetricDevice &&
                    candidate.IconDeviceId == "device-b" && candidate.Theme == AppSettings.ThemeLight,
                    "actual save candidate contains selected controls");
                Check(candidate.UseCustomTrayBoxColor && candidate.TrayBoxColor == "#005853" && candidate.TrayTextColor == "#000000",
                    "actual save candidate contains selected colors");
                Check(candidate.CriticalThreshold == 20 && candidate.LowThreshold == 20,
                    "actual save candidate contains adjusted thresholds");
                Check(settings.RefreshSeconds == 60 && settings.Theme == AppSettings.ThemeDark,
                    "editing settings has not saved changes");
                TestIsolatedPersistence(candidate);

                typeof(SettingsForm).GetMethod("RestoreDefaults", PrivateInstance).Invoke(form, null);
                Check(((RadioButton)Field(view, "lowest")).IsChecked == true &&
                    ((Wpf.Ui.Controls.NumberBox)Field(view, "refresh")).Value == 60,
                    "restore defaults updates the WPF controls");
                Check(settings.RefreshSeconds == 60 && settings.Theme == AppSettings.ThemeDark,
                    "restore defaults does not save without user action");

                source.RootVisual = null;
            }
        }
        TestSmallWindow(settings, snapshot, outputDirectory);
        TestUpdateHandoffWithSettings();
        Console.WriteLine("Settings WPF headless checks passed.");
    }

    private static void TestUpdateHandoffWithSettings()
    {
        string tempRoot = Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar);
        string directory = Path.GetFullPath(Path.Combine(tempRoot, "BluetoothSettingsSaveTests-" + Guid.NewGuid().ToString("N")));
        Check(string.Equals(Path.GetDirectoryName(directory), tempRoot, StringComparison.OrdinalIgnoreCase), "isolated settings path stays in temp");
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "settings.json");
        try
        {
            using (SettingsForm form = new SettingsForm(new AppSettings(), new UsageSnapshot()))
            {
                int handoffs = 0;
                Func<bool> dirty = delegate { return !form.IsDisposed && form.HasUnsavedChanges; };
                Action handoff = delegate { handoffs++; };
                Check(!dirty(), "update preparation can start with clean settings");
                FrameworkElement view = (FrameworkElement)Field(form, "settingsView");
                Wpf.Ui.Controls.NumberBox refresh = (Wpf.Ui.Controls.NumberBox)Field(view, "refresh");
                refresh.Value = 120;
                Check(!TrayAppContext.TryHandoffWhenReady(dirty, delegate { return false; }, handoff) && handoffs == 0,
                    "an edit made during update preparation defers restart");

                bool appliedWhileDirty = false;
                form.SettingsApplied += delegate { appliedWhileDirty = form.HasUnsavedChanges; };
                MethodInfo saveToPath = typeof(AppSettings).GetMethod("SaveToPath", PrivateInstance);
                bool saved = form.TryApplyToSettingsUsing(delegate(AppSettings candidate)
                {
                    saveToPath.Invoke(candidate, new object[] { path });
                });
                Check(saved && File.Exists(path) && appliedWhileDirty && !form.HasUnsavedChanges,
                    "successful isolated save applies settings before clearing pending edits");
                Check(TrayAppContext.TryHandoffWhenReady(dirty, delegate { return false; }, handoff) && handoffs == 1,
                    "restart resumes after settings are saved");
            }

            using (SettingsForm form = new SettingsForm(new AppSettings(), new UsageSnapshot()))
            {
                int handoffs = 0;
                Func<bool> dirty = delegate { return !form.IsDisposed && form.HasUnsavedChanges; };
                bool closeNotified = false;
                form.FormClosed += delegate { closeNotified = true; };
                FrameworkElement view = (FrameworkElement)Field(form, "settingsView");
                ((Wpf.Ui.Controls.NumberBox)Field(view, "refresh")).Value = 120;
                Check(!TrayAppContext.TryHandoffWhenReady(dirty, delegate { return false; }, delegate { handoffs++; }),
                    "unsaved settings defer restart before closing");
                IntPtr hiddenHandle = form.Handle;
                form.Close();
                Check(hiddenHandle != IntPtr.Zero && form.IsDisposed && closeNotified && TrayAppContext.TryHandoffWhenReady(dirty, delegate { return false; }, delegate { handoffs++; }) && handoffs == 1,
                    "closing and discarding settings lets restart resume");
            }
        }
        finally
        {
            if (File.Exists(path)) File.Delete(path);
            string backup = path + ".bak";
            if (File.Exists(backup)) File.Delete(backup);
            Directory.Delete(directory);
        }
    }

    private static void TestUnsavedChanges(SettingsForm form, FrameworkElement view, UsageSnapshot snapshot)
    {
        int changes = 0;
        form.UnsavedChangesChanged += delegate { changes++; };
        Check(!form.HasUnsavedChanges, "initial settings are clean");
        form.UpdateDevices(snapshot);
        Check(!form.HasUnsavedChanges && changes == 0, "refreshing the device list leaves settings clean");
        UsageSnapshot reordered = new UsageSnapshot();
        reordered.Devices.Add(new LimitWindow { DeviceId = "device-a", DisplayName = "A mouse", BatteryPercent = 20 });
        reordered.Devices.Add(new LimitWindow { DeviceId = "headset", DisplayName = "Renamed headset", BatteryPercent = 40 });
        form.UpdateDevices(reordered);
        Check(!form.HasUnsavedChanges && changes == 0, "device reordering and updated labels do not create unsaved edits");
        form.UpdateDevices(snapshot);
        MethodInfo setModalEdit = typeof(SettingsForm).GetMethod("SetModalSettingsEditInProgress", PrivateInstance);
        setModalEdit.Invoke(form, new object[] { true });
        Check(form.HasUnsavedChanges, "an open color picker prevents a restart before a selection is committed");
        setModalEdit.Invoke(form, new object[] { false });
        Check(!form.HasUnsavedChanges, "canceling the color picker clears the pending edit");

        Click(FindByName<Wpf.Ui.Controls.Button>(view, "Dark teal background"));
        Check(form.HasUnsavedChanges, "choosing a fixed color is dirty");
        Click((Wpf.Ui.Controls.Button)Field(view, "autoColor"));
        Check(!form.HasUnsavedChanges, "returning to automatic background is clean");

        Wpf.Ui.Controls.NumberBox preview = (Wpf.Ui.Controls.NumberBox)Field(view, "previewPercent");
        preview.Value = 75;
        Check(!form.HasUnsavedChanges, "preview-only edits are not unsaved settings");
        preview.Value = 50;

        Wpf.Ui.Controls.NumberBox refresh = (Wpf.Ui.Controls.NumberBox)Field(view, "refresh");
        string originalText = refresh.Text;
        refresh.Text = "invalid";
        Check(form.HasUnsavedChanges, "invalid numeric text is protected before it is committed");
        refresh.Text = originalText;
        Check(!form.HasUnsavedChanges, "reverting uncommitted numeric text clears the dirty state");
        refresh.Text = "90";
        Check(form.HasUnsavedChanges, "uncommitted numeric text is protected");
        refresh.Text = originalText;
        Check(!form.HasUnsavedChanges, "reverting numeric text clears the dirty state");
        refresh.Value = 120;
        Check(form.HasUnsavedChanges, "a committed numeric edit is dirty");
        refresh.Value = 60;
        Check(!form.HasUnsavedChanges, "reverting a committed numeric edit is clean");

        ComboBox devices = (ComboBox)Field(view, "device");
        devices.SelectedIndex = 1;
        Check(form.HasUnsavedChanges, "selecting another device is dirty");
        form.UpdateDevices(snapshot);
        Check(form.HasUnsavedChanges, "refreshing devices preserves an unsaved selection");
        devices.SelectedIndex = 0;
        Check(!form.HasUnsavedChanges, "restoring the selected device is clean");
        ((System.Windows.Forms.ComboBox)Field(form, "deviceCombo")).SelectedIndex = -1;
        Check(form.HasUnsavedChanges, "a missing required device selection is protected");
        form.UpdateDevices(snapshot);
        Check(!form.HasUnsavedChanges, "restoring the original device clears the invalid selection");

        CheckBox startup = (CheckBox)Field(view, "start");
        bool originalStartup = startup.IsChecked == true;
        startup.IsChecked = !originalStartup;
        Check(form.HasUnsavedChanges, "changing a checkbox is dirty");
        startup.IsChecked = originalStartup;
        Check(!form.HasUnsavedChanges, "reverting a checkbox is clean");
        Check(changes >= 8, "dirty-state changes raise notifications");
    }

    private static void TestSmallWindow(AppSettings settings, UsageSnapshot snapshot, string outputDirectory)
    {
        UsageSnapshot smallSnapshot = new UsageSnapshot();
        foreach (LimitWindow item in snapshot.Devices) smallSnapshot.Devices.Add(item);
        string longName = "Wireless device with a particularly long product name that must remain completely readable";
        smallSnapshot.Devices.Add(new LimitWindow { DeviceId = "long-a", DisplayName = longName, BatteryPercent = 50 });
        smallSnapshot.Devices.Add(new LimitWindow { DeviceId = "long-b", DisplayName = longName, BatteryPercent = 60 });
        using (SettingsForm form = new SettingsForm(settings, smallSnapshot))
        {
            FrameworkElement view = (FrameworkElement)Field(form, "settingsView");
            ((System.Windows.Forms.Integration.ElementHost)form.Controls.Find("settingsWpfHost", true)[0]).Child = null;
            HwndSourceParameters parameters = new HwndSourceParameters("Headless small settings checks")
            {
                Width = 640, Height = 420, PositionX = -30000, PositionY = -30000, WindowStyle = 0
            };
            using (HwndSource source = new HwndSource(parameters))
            {
                source.RootVisual = view;
                Layout(view, 640, 420);
                TextBlock label = FindText(view, "Background color");
                Check(label != null && label.ActualWidth >= TextWidth(label), "background label remains readable in a small window");
                ComboBox devices = (ComboBox)Field(view, "device");
                AssertSelectedDeviceFits(devices, "headset at small window size");
                ScrollViewer scroll = FindByName<ScrollViewer>(view, "Settings content");
                Check(scroll.HorizontalScrollBarVisibility == ScrollBarVisibility.Disabled && scroll.ScrollableWidth < 0.1,
                    "small settings window has no horizontal scrolling");
                Render(view, 640, 420, outputDirectory, "settings-small.png");
                int duplicateIndex = -1;
                for (int i = 0; i < devices.Items.Count; i++)
                    if (devices.Items[i].ToString() == longName + " #2") duplicateIndex = i;
                Check(duplicateIndex >= 0, "duplicate device has a disambiguating suffix");
                devices.SelectedIndex = duplicateIndex;
                Layout(view, 640, 420);
                AssertSelectedDeviceFits(devices, "long duplicate device at small window size");
                Render(view, 640, 420, outputDirectory, "settings-small-long-device.png");
                source.RootVisual = null;
            }
        }
    }

    private static void AssertSelectedDeviceFits(ComboBox devices, string context)
    {
        string selected = devices.SelectedItem == null ? "" : devices.SelectedItem.ToString();
        TextBlock text = FindText(devices, selected);
        Check(text != null && text.TextWrapping == TextWrapping.Wrap, context + " wraps the selected value");
        FormattedText measure = new FormattedText(text.Text, CultureInfo.CurrentCulture, text.FlowDirection,
            new Typeface(text.FontFamily, text.FontStyle, text.FontWeight, text.FontStretch), text.FontSize, text.Foreground, 1.0);
        measure.MaxTextWidth = text.ActualWidth;
        Check(text.ActualHeight + 1 >= measure.Height, context + " shows all wrapped lines");
        Rect bounds = text.TransformToAncestor(devices).TransformBounds(new Rect(text.RenderSize));
        Check(bounds.Left >= 0 && bounds.Top >= 0 && bounds.Right <= devices.ActualWidth + 1 && bounds.Bottom <= devices.ActualHeight + 1,
            context + " stays within the selector");
    }

    private static void TestIsolatedPersistence(AppSettings candidate)
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatterySettingsTest-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string path = Path.Combine(directory, "settings.json");
        try
        {
            typeof(AppSettings).GetMethod("SaveToPath", PrivateInstance).Invoke(candidate, new object[] { path });
            AppSettings loaded = (AppSettings)typeof(AppSettings).GetMethod("LoadFromPath", PrivateStatic).Invoke(null, new object[] { path });
            Check(loaded.IconDeviceId == candidate.IconDeviceId && loaded.RefreshSeconds == candidate.RefreshSeconds &&
                loaded.TrayBoxColor == candidate.TrayBoxColor && loaded.Theme == candidate.Theme,
                "actual save candidate round-trips through an isolated file");
        }
        finally
        {
            if (File.Exists(path)) File.Delete(path);
            Directory.Delete(directory);
        }
    }

    private static void Layout(FrameworkElement view, int width, int height)
    {
        view.Measure(new Size(width, height));
        view.Arrange(new Rect(0, 0, width, height));
        view.UpdateLayout();
        DispatcherFrame frame = new DispatcherFrame();
        DispatcherTimer timer = new DispatcherTimer(DispatcherPriority.ApplicationIdle);
        timer.Interval = TimeSpan.FromMilliseconds(300);
        timer.Tick += delegate { timer.Stop(); frame.Continue = false; };
        timer.Start();
        Dispatcher.PushFrame(frame);
        view.UpdateLayout();
    }

    private static void Render(FrameworkElement view, int width, int height, string directory, string name)
    {
        RenderTargetBitmap render = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32);
        render.Render(view);
        if (directory == null) return;
        PngBitmapEncoder encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(render));
        using (FileStream file = File.Create(Path.Combine(directory, name))) encoder.Save(file);
    }

    private static void AssertNumberInputFits(Wpf.Ui.Controls.NumberBox number)
    {
        FrameworkElement textArea = number.Template.FindName("PART_ContentHost", number) as FrameworkElement;
        Check(textArea != null && textArea.ActualWidth >= 26, "number input has room to display its value");
    }

    private static byte[] FirstPixel(BitmapSource image)
    {
        FormatConvertedBitmap converted = new FormatConvertedBitmap(image, PixelFormats.Bgra32, null, 0);
        byte[] pixel = new byte[4];
        converted.CopyPixels(new Int32Rect(0, 0, 1, 1), pixel, 4, 0);
        return pixel;
    }

    private static double TextWidth(TextBlock text)
    {
        FormattedText measured = new FormattedText(text.Text, CultureInfo.CurrentCulture, text.FlowDirection,
            new Typeface(text.FontFamily, text.FontStyle, text.FontWeight, text.FontStretch), text.FontSize, text.Foreground, 1.0);
        return measured.WidthIncludingTrailingWhitespace;
    }

    private static TextBlock FindText(DependencyObject parent, string value)
    {
        TextBlock text = parent as TextBlock;
        if (text != null && text.Text == value && text.ActualWidth > 0) return text;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            TextBlock child = FindText(VisualTreeHelper.GetChild(parent, i), value);
            if (child != null) return child;
        }
        return null;
    }

    private static T FindByName<T>(DependencyObject parent, string name) where T : DependencyObject
    {
        T match = parent as T;
        if (match != null && AutomationProperties.GetName(match) == name) return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            T child = FindByName<T>(VisualTreeHelper.GetChild(parent, i), name);
            if (child != null) return child;
        }
        return null;
    }

    private static void Click(Wpf.Ui.Controls.Button button)
    {
        Check(button != null, "button exists");
        button.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
    }

    private static void ShowPage(object view, int page)
    {
        view.GetType().GetMethod("ShowPage", PrivateInstance).Invoke(view, new object[] { page });
    }

    private static object Field(object target, string name)
    {
        FieldInfo field = target.GetType().GetField(name, PrivateInstance);
        Check(field != null, "field exists: " + name);
        return field.GetValue(target);
    }

    private static void Check(bool condition, string description)
    {
        if (!condition) throw new Exception(description);
    }
}
