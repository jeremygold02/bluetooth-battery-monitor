using System;
using System.Collections.Generic;
using System.IO;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Forms.Integration;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Wpf.Ui.Appearance;
using Wpf.Ui.Markup;
using WpfButton = Wpf.Ui.Controls.Button;
using WpfNumberBox = Wpf.Ui.Controls.NumberBox;
using WinForms = System.Windows.Forms;

namespace CodexUsageTray
{
    internal sealed partial class SettingsForm
    {
        private void BuildWpfSettingsView()
        {
            foreach (WinForms.Control control in Controls) control.Visible = false;
            AcceptButton = null;
            CancelButton = null;
            settingsView = new SettingsView(this);
            ElementHost host = new ElementHost();
            host.Name = "settingsWpfHost";
            host.Dock = WinForms.DockStyle.Fill;
            host.Child = settingsView;
            Controls.Add(host);
            host.BringToFront();
        }

        private sealed class SettingsView : UserControl
        {
            private readonly SettingsForm owner;
            private readonly Grid root;
            private StackPanel content;
            private readonly StackPanel[] pages = new StackPanel[4];
            private readonly WpfButton[] navigation = new WpfButton[4];
            private readonly List<Border> cards = new List<Border>();
            private readonly List<TextBlock> muted = new List<TextBlock>();
            private readonly List<WpfButton> swatches = new List<WpfButton>();
            private RadioButton lowest, average, specific;
            private ComboBox device, theme;
            private CheckBox background, start, notifications, colorByLevel, moreDevices, details, updated;
            private WpfNumberBox refresh, critical, low, previewPercent;
            private Image preview;
            private TextBlock backgroundValue, textValue, contrast, updateStatus;
            private WrapPanel backgroundChoices, textChoices;
            private WpfButton autoColor, saveAction, checkUpdates;
            private Border previewFrame;
            private bool syncing;
            private int selectedPage = 2;
            private bool themeHighContrast, themeDark;
            private ThemesDictionary themeResources;
            private ApplicationTheme currentTheme = ApplicationTheme.Unknown;
            private Brush foreground, secondary, surface, border, baseBackground, sideBackground, accent;

            public SettingsView(SettingsForm owner)
            {
                this.owner = owner;
                FontFamily = new FontFamily("Segoe UI");
                FontSize = 15;
                root = new Grid();
                root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(205) });
                root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
                root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                Content = root;

                Border sidebar = new Border { Padding = new Thickness(12, 24, 12, 16) };
                sidebar.SetResourceReference(Border.BackgroundProperty, "SettingsSidebarBrush");
                Grid.SetRowSpan(sidebar, 2);
                root.Children.Add(sidebar);
                StackPanel navPanel = new StackPanel();
                sidebar.Child = navPanel;
                string[] names = { "General", "Battery & alerts", "Tray icon", "Appearance" };
                string[] glyphs = { "\uE713", "\uE850", "\uE7F4", "\uE790" };
                for (int i = 0; i < names.Length; i++)
                {
                    int page = i;
                    WpfButton button = new WpfButton { HorizontalContentAlignment = HorizontalAlignment.Left,
                        HorizontalAlignment = HorizontalAlignment.Stretch, Padding = new Thickness(12, 9, 12, 9),
                        Margin = new Thickness(0, 0, 0, 5), MinHeight = 43, BorderThickness = new Thickness(0) };
                    StackPanel label = new StackPanel { Orientation = Orientation.Horizontal };
                    label.Children.Add(new TextBlock { Text = glyphs[i], FontFamily = new FontFamily("Segoe MDL2 Assets"),
                        FontSize = 17, Width = 28, VerticalAlignment = VerticalAlignment.Center });
                    label.Children.Add(new TextBlock { Text = names[i], FontSize = 15, VerticalAlignment = VerticalAlignment.Center });
                    button.Content = label;
                    AutomationProperties.SetName(button, names[i]);
                    button.Click += delegate { ShowPage(page); };
                    navigation[i] = button;
                    navPanel.Children.Add(button);
                }

                ScrollViewer scroll = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                    HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, Padding = new Thickness(26, 20, 26, 16),
                    CanContentScroll = false };
                AutomationProperties.SetName(scroll, "Settings content");
                Grid.SetColumn(scroll, 1);
                root.Children.Add(scroll);
                Grid pageHost = new Grid();
                scroll.Content = pageHost;
                for (int i = 0; i < pages.Length; i++)
                {
                    pages[i] = new StackPanel { Visibility = Visibility.Collapsed };
                    pageHost.Children.Add(pages[i]);
                }
                Border footer = new Border { Padding = new Thickness(26, 12, 26, 12), BorderThickness = new Thickness(0, 1, 0, 0) };
                footer.SetResourceReference(Border.BorderBrushProperty, "SettingsBorderBrush");
                Grid.SetColumn(footer, 1);
                Grid.SetRow(footer, 1);
                root.Children.Add(footer);
                DockPanel actions = new DockPanel { LastChildFill = false };
                footer.Child = actions;
                WpfButton defaultsButton = ActionButton("Defaults", false);
                defaultsButton.Click += delegate { owner.RestoreDefaults(); };
                DockPanel.SetDock(defaultsButton, System.Windows.Controls.Dock.Left);
                actions.Children.Add(defaultsButton);
                WpfButton saveButton = ActionButton("Save", true);
                saveAction = saveButton;
                saveButton.IsDefault = true;
                saveButton.Click += delegate { if (owner.TryApplyToSettings()) owner.Close(); };
                DockPanel.SetDock(saveButton, System.Windows.Controls.Dock.Right);
                actions.Children.Add(saveButton);
                WpfButton cancelButton = ActionButton("Cancel", false);
                cancelButton.Margin = new Thickness(0, 0, 10, 0);
                cancelButton.IsCancel = true;
                cancelButton.Click += delegate { owner.Close(); };
                DockPanel.SetDock(cancelButton, System.Windows.Controls.Dock.Right);
                actions.Children.Add(cancelButton);
                RefreshTheme();
                content = pages[0]; BuildGeneral();
                content = pages[1]; BuildAlerts();
                content = pages[2]; BuildTray();
                content = pages[3]; BuildAppearance();
                foreach (StackPanel page in pages)
                    if (page.Children.Count > 0) ((FrameworkElement)page.Children[page.Children.Count - 1]).Margin = new Thickness(0);
                RefreshFromOwner();
                ShowPage(selectedPage);
            }

            private static WpfButton ActionButton(string text, bool primary)
            {
                WpfButton button = new WpfButton { Content = text, MinWidth = 100, MinHeight = 36,
                    Padding = new Thickness(16, 7, 16, 7), HorizontalContentAlignment = HorizontalAlignment.Center };
                if (primary)
                {
                    button.Appearance = Wpf.Ui.Controls.ControlAppearance.Primary;
                    button.Background = new SolidColorBrush(Color.FromRgb(0, 105, 190));
                    button.BorderBrush = new SolidColorBrush(Color.FromRgb(38, 151, 235));
                    button.Foreground = Brushes.White;
                }
                return button;
            }

            public void RefreshTheme()
            {
                bool highContrast = WinForms.SystemInformation.HighContrast;
                bool dark = !highContrast && AppSettings.IsDarkTheme(owner.GetSelectedTheme());
                ApplyViewTheme(highContrast, dark);
            }

            private void ApplyViewTheme(bool highContrast, bool dark)
            {
                themeHighContrast = highContrast;
                themeDark = dark;
                ApplicationTheme requestedTheme = highContrast ? ApplicationTheme.HighContrast : dark ? ApplicationTheme.Dark : ApplicationTheme.Light;
                if (themeResources == null)
                {
                    themeResources = new ThemesDictionary();
                    SetThemeSource(requestedTheme);
                    Resources.MergedDictionaries.Add(themeResources);
                    Resources.MergedDictionaries.Add(new ControlsDictionary());
                    currentTheme = requestedTheme;
                }
                else if (currentTheme != requestedTheme)
                {
                    SetThemeSource(requestedTheme);
                    currentTheme = requestedTheme;
                }
                baseBackground = highContrast ? SystemColors.WindowBrush : dark ? Brush(22, 22, 22) : Brush(246, 248, 248);
                sideBackground = highContrast ? SystemColors.ControlBrush : dark ? Brush(18, 18, 18) : Brush(237, 241, 241);
                surface = highContrast ? SystemColors.WindowBrush : dark ? Brush(34, 34, 34) : Brush(251, 251, 251);
                foreground = highContrast ? SystemColors.WindowTextBrush : dark ? Brush(220, 220, 220) : Brush(24, 30, 31);
                secondary = highContrast ? SystemColors.WindowTextBrush : dark ? Brush(192, 192, 192) : Brush(82, 90, 92);
                border = highContrast ? SystemColors.WindowTextBrush : dark ? Brush(78, 78, 78) : Brush(190, 198, 199);
                accent = highContrast ? SystemColors.HighlightBrush : Brush(22, 133, 220);
                if (!highContrast)
                {
                    Resources["AccentFillColorDefaultBrush"] = accent;
                    Resources["AccentFillColorSecondaryBrush"] = accent;
                    Resources["AccentFillColorTertiaryBrush"] = accent;
                    Resources["SystemAccentColorPrimaryBrush"] = accent;
                    Resources["SystemAccentColorSecondaryBrush"] = accent;
                    Resources["RadioButtonOuterEllipseCheckedStroke"] = accent;
                    Resources["RadioButtonCheckGlyphFill"] = surface;
                    Resources["CheckBoxCheckBackgroundFillChecked"] = accent;
                    Resources["CheckBoxCheckBorderBrush"] = accent;
                }
                else
                {
                    string[] keys = { "AccentFillColorDefaultBrush", "AccentFillColorSecondaryBrush", "AccentFillColorTertiaryBrush",
                        "SystemAccentColorPrimaryBrush", "SystemAccentColorSecondaryBrush", "RadioButtonOuterEllipseCheckedStroke",
                        "RadioButtonCheckGlyphFill", "CheckBoxCheckBackgroundFillChecked", "CheckBoxCheckBorderBrush" };
                    foreach (string key in keys) Resources.Remove(key);
                }
                Resources["SettingsBackgroundBrush"] = baseBackground;
                Resources["SettingsSidebarBrush"] = sideBackground;
                Resources["SettingsSurfaceBrush"] = surface;
                Resources["SettingsForegroundBrush"] = foreground;
                Resources["SettingsSecondaryBrush"] = secondary;
                Resources["SettingsBorderBrush"] = border;
                root.SetResourceReference(Panel.BackgroundProperty, "SettingsBackgroundBrush");
                SetResourceReference(ForegroundProperty, "SettingsForegroundBrush");
                foreach (Border card in cards) { card.Background = surface; card.BorderBrush = border; }
                foreach (TextBlock text in muted) text.Foreground = secondary;
                if (previewFrame != null)
                {
                    previewFrame.Background = highContrast ? SystemColors.WindowBrush : dark ? Brush(20, 20, 20) : Brush(232, 232, 232);
                    previewFrame.BorderBrush = border;
                }
                saveAction.Background = highContrast ? SystemColors.HighlightBrush : Brush(0, 105, 190);
                saveAction.BorderBrush = highContrast ? SystemColors.HighlightBrush : Brush(38, 151, 235);
                saveAction.Foreground = highContrast ? SystemColors.HighlightTextBrush : Brushes.White;
                UpdateNavigation();
            }

            private void SetThemeSource(ApplicationTheme theme)
            {
                if (theme == ApplicationTheme.HighContrast)
                {
                    Color window = SystemColors.WindowColor;
                    bool lightWindow = (window.R * 299 + window.G * 587 + window.B * 114) >= 128000;
                    string name = lightWindow ? "HCWhite" : "HCBlack";
                    themeResources.Source = new Uri("pack://application:,,,/Wpf.Ui;component/Resources/Theme/" + name + ".xaml", UriKind.Absolute);
                }
                else themeResources.Theme = theme;
            }

            private static SolidColorBrush Brush(int r, int g, int b) { return new SolidColorBrush(Color.FromRgb((byte)r, (byte)g, (byte)b)); }

            private void UpdateNavigation()
            {
                for (int i = 0; i < navigation.Length; i++)
                {
                    bool selected = i == selectedPage;
                    navigation[i].Background = selected ? (themeHighContrast ? SystemColors.HighlightBrush : themeDark ? Brush(25, 66, 100) : Brush(203, 228, 250)) : sideBackground;
                    navigation[i].Foreground = selected && themeHighContrast ? SystemColors.HighlightTextBrush : foreground;
                }
            }

            private TextBlock Text(string value, double size, bool bold, bool mutedText)
            {
                TextBlock text = new TextBlock { Text = value, FontSize = Math.Max(14, size), FontWeight = bold ? FontWeights.SemiBold : FontWeights.Normal,
                    TextWrapping = TextWrapping.Wrap, Foreground = mutedText ? secondary : foreground };
                if (mutedText) muted.Add(text);
                return text;
            }

            private StackPanel AddCard()
            {
                Border card = new Border { Background = surface, BorderBrush = border, BorderThickness = new Thickness(1),
                    CornerRadius = new CornerRadius(7), Padding = new Thickness(20, 14, 20, 14), Margin = new Thickness(0, 0, 0, 12) };
                StackPanel body = new StackPanel();
                card.Child = body;
                cards.Add(card);
                content.Children.Add(card);
                return body;
            }

            private void Heading(string title, string description)
            {
                content.Children.Add(Text(title, 25, true, false));
                TextBlock summary = Text(description, 14, false, true);
                summary.Margin = new Thickness(0, 3, 0, 18);
                content.Children.Add(summary);
            }

            private void ShowPage(int page)
            {
                if (page < 0 || page > 3) return;
                selectedPage = page;
                for (int i = 0; i < pages.Length; i++)
                    pages[i].Visibility = i == page ? Visibility.Visible : Visibility.Collapsed;
                RefreshFromOwner();
                UpdateNavigation();
            }

            private static void Space(Panel panel, double amount) { panel.Children.Add(new Border { Height = amount }); }

            private void BuildGeneral()
            {
                Heading("General", "Choose how often devices are checked and how the app starts.");
                StackPanel polling = AddCard();
                polling.Children.Add(Text("Battery checks", 15, true, false));
                polling.Children.Add(Text("Check connected devices at this interval.", 13, false, true));
                Space(polling, 10);
                StackPanel row = new StackPanel { Orientation = Orientation.Horizontal };
                refresh = NumberBox(30, 3600, "Battery check interval in seconds");
                refresh.ValueChanged += delegate { if (!syncing && refresh.Value.HasValue) owner.refreshNumeric.Value = (decimal)refresh.Value.Value; };
                row.Children.Add(refresh);
                row.Children.Add(new TextBlock { Text = " seconds", VerticalAlignment = VerticalAlignment.Center, Foreground = secondary });
                polling.Children.Add(row);
                StackPanel startup = AddCard();
                startup.Children.Add(Text("Startup", 15, true, false));
                Space(startup, 8);
                start = Check("Start with Windows", delegate(bool value) { owner.startWithWindowsCheckBox.Checked = value; });
                startup.Children.Add(start);
                StackPanel about = AddCard();
                about.Children.Add(Text("About", 15, true, false));
                Space(about, 8);
                about.Children.Add(Text("Version " + AppVersion.Current, 13, false, true));
                updateStatus = Text("", 13, false, true);
                about.Children.Add(updateStatus);
                Space(about, 10);
                checkUpdates = ActionButton("Check for updates", false);
                checkUpdates.HorizontalAlignment = HorizontalAlignment.Left;
                AutomationProperties.SetName(checkUpdates, "Check for updates");
                checkUpdates.Click += delegate
                {
                    if (checkUpdates.IsEnabled && owner.UpdateCheckRequested != null)
                        owner.UpdateCheckRequested(owner, EventArgs.Empty);
                };
                about.Children.Add(checkUpdates);
            }

            private void BuildAlerts()
            {
                Heading("Battery & alerts", "Notifications and battery warning thresholds.");
                StackPanel alerts = AddCard();
                alerts.Children.Add(Text("Battery notifications", 15, true, false));
                alerts.Children.Add(Text("Show an alert when a device reaches a threshold.", 13, false, true));
                Space(alerts, 8);
                notifications = Check("Notify at thresholds", delegate(bool value) { owner.thresholdNotificationsCheckBox.Checked = value; });
                alerts.Children.Add(notifications);
                StackPanel levels = AddCard();
                levels.Children.Add(Text("Battery thresholds", 15, true, false));
                Space(levels, 10);
                WrapPanel row = new WrapPanel();
                row.Children.Add(Text("Critical level  ", 14, false, true));
                critical = NumberBox(1, 99, "Critical threshold percent");
                critical.ValueChanged += delegate { if (!syncing && critical.Value.HasValue) { owner.criticalNumeric.Value = (decimal)critical.Value.Value; RefreshNumbers(); } };
                row.Children.Add(critical);
                row.Children.Add(Text(" %     Low level  ", 14, false, true));
                low = NumberBox(1, 99, "Low threshold percent");
                low.ValueChanged += delegate { if (!syncing && low.Value.HasValue) { owner.lowNumeric.Value = (decimal)low.Value.Value; RefreshNumbers(); } };
                row.Children.Add(low);
                row.Children.Add(Text(" %", 14, false, true));
                levels.Children.Add(row);
            }

            private void BuildTray()
            {
                Heading("Tray icon", "Customize how the battery level is shown in the system tray.");
                StackPanel metric = AddCard();
                lowest = MetricOption(metric, "Lowest battery", "Show the lowest battery level across your devices.");
                average = MetricOption(metric, "Average battery", "Show the average battery level across your devices.");
                specific = MetricOption(metric, "Specific device", "Show the battery level for a chosen device.");
                lowest.Checked += delegate { if (!syncing) { owner.weeklyRadio.Checked = true; owner.fiveHourRadio.Checked = false; owner.deviceRadio.Checked = false; RefreshDevices(); } };
                average.Checked += delegate { if (!syncing) { owner.fiveHourRadio.Checked = true; owner.weeklyRadio.Checked = false; owner.deviceRadio.Checked = false; RefreshDevices(); } };
                specific.Checked += delegate { if (!syncing) { owner.deviceRadio.Checked = true; owner.weeklyRadio.Checked = false; owner.fiveHourRadio.Checked = false; RefreshDevices(); } };
                device = new ComboBox { MinHeight = 32, Margin = new Thickness(28, 0, 0, 0), MaxWidth = 500,
                    HorizontalAlignment = HorizontalAlignment.Left, MinWidth = 260 };
                FrameworkElementFactory deviceLabel = new FrameworkElementFactory(typeof(TextBlock));
                deviceLabel.SetBinding(TextBlock.TextProperty, new System.Windows.Data.Binding());
                deviceLabel.SetValue(TextBlock.TextWrappingProperty, TextWrapping.Wrap);
                deviceLabel.SetValue(TextBlock.TextTrimmingProperty, TextTrimming.None);
                device.ItemTemplate = new DataTemplate { VisualTree = deviceLabel };
                AutomationProperties.SetName(device, "Device for tray icon");
                device.SelectionChanged += delegate { if (!syncing && device.SelectedIndex >= 0) owner.deviceCombo.SelectedIndex = device.SelectedIndex; };
                device.SelectionChanged += delegate { UpdateDeviceDescription(); };
                metric.Children.Add(device);

                StackPanel previewCard = AddCard();
                Grid previewGrid = new Grid();
                previewGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(155) });
                previewGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                previewCard.Children.Add(previewGrid);
                StackPanel previewColumn = new StackPanel { Margin = new Thickness(0, 0, 18, 0) };
                previewColumn.Children.Add(Text("LIVE PREVIEW", 12, true, true));
                preview = new Image { Width = 36, Height = 36, HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center };
                RenderOptions.SetBitmapScalingMode(preview, BitmapScalingMode.NearestNeighbor);
                AutomationProperties.SetName(preview, "Live tray preview");
                previewFrame = new Border { Width = 52, Height = 52, Margin = new Thickness(0, 10, 0, 10),
                    HorizontalAlignment = HorizontalAlignment.Left, BorderBrush = border, BorderThickness = new Thickness(1),
                    CornerRadius = new CornerRadius(5), Background = themeHighContrast ? SystemColors.WindowBrush : themeDark ? Brush(20, 20, 20) : Brush(232, 232, 232), Child = preview };
                previewColumn.Children.Add(previewFrame);
                StackPanel sample = new StackPanel();
                sample.Children.Add(Text("Battery preview", 14, false, true));
                previewPercent = NumberBox(0, 100, "Preview battery percent");
                previewPercent.Width = 130;
                previewPercent.ValueChanged += delegate { if (!syncing && previewPercent.Value.HasValue) owner.previewPercentNumeric.Value = (decimal)previewPercent.Value.Value; };
                sample.Children.Add(previewPercent);
                previewColumn.Children.Add(sample);
                previewGrid.Children.Add(previewColumn);
                StackPanel colors = new StackPanel { Margin = new Thickness(16, 0, 0, 0) };
                Grid.SetColumn(colors, 1);
                previewGrid.Children.Add(colors);
                background = Check("Show tray background", delegate(bool value) { owner.showTrayBoxCheckBox.Checked = value; });
                colors.Children.Add(background);
                Space(colors, 5);
                WrapPanel bgButtons = new WrapPanel { Margin = new Thickness(0, 0, 0, 2) };
                backgroundChoices = bgButtons;
                bgButtons.Children.Add(new TextBlock { Text = "Background color", Width = 130, FontSize = 14, VerticalAlignment = VerticalAlignment.Center, Foreground = secondary });
                autoColor = SmallButton("Auto");
                autoColor.Click += delegate { owner.useCustomTrayBoxColor = false; owner.UpdatePreviews(); };
                bgButtons.Children.Add(autoColor);
                AddSwatch(bgButtons, System.Drawing.Color.Black, true, "Black background");
                AddSwatch(bgButtons, System.Drawing.Color.FromArgb(73, 81, 88), true, "Slate background");
                AddSwatch(bgButtons, System.Drawing.Color.FromArgb(0, 88, 83), true, "Dark teal background");
                AddSwatch(bgButtons, System.Drawing.Color.FromArgb(0, 194, 186), true, "Teal background");
                WpfButton customBackground = SmallButton("Custom…");
                customBackground.Click += delegate { owner.PickTrayBoxColor(); };
                bgButtons.Children.Add(customBackground);
                colors.Children.Add(bgButtons);
                backgroundValue = Text("", 12, false, true);
                WrapPanel textButtons = new WrapPanel { Margin = new Thickness(0, 0, 0, 2) };
                textChoices = textButtons;
                textButtons.Children.Add(new TextBlock { Text = "Text color", Width = 130, FontSize = 14, VerticalAlignment = VerticalAlignment.Center, Foreground = secondary });
                AddSwatch(textButtons, System.Drawing.Color.White, false, "White text");
                AddSwatch(textButtons, System.Drawing.Color.LightGray, false, "Light gray text");
                AddSwatch(textButtons, System.Drawing.Color.Gray, false, "Gray text");
                AddSwatch(textButtons, System.Drawing.Color.Black, false, "Black text");
                WpfButton customText = SmallButton("Custom…");
                customText.Click += delegate { owner.PickTrayTextColor(); };
                textButtons.Children.Add(customText);
                colors.Children.Add(textButtons);
                textValue = Text("", 12, false, true);
                contrast = Text("", 12, false, true);
                contrast.Margin = new Thickness(0, 2, 0, 0);
                colors.Children.Add(contrast);

                StackPanel themeCard = AddCard();
                Grid themeRow = new Grid();
                themeRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                themeRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                StackPanel themeCaption = new StackPanel { Margin = new Thickness(0, 0, 12, 0) };
                themeCaption.Children.Add(Text("Theme", 15, true, false));
                themeCaption.Children.Add(Text("Choose the app color theme.", 14, false, true));
                themeRow.Children.Add(themeCaption);
                theme = new ComboBox { Width = 220, HorizontalAlignment = HorizontalAlignment.Right,
                    VerticalAlignment = VerticalAlignment.Center, MinHeight = 32 };
                theme.Items.Add(AppSettings.ThemeSystem);
                theme.Items.Add(AppSettings.ThemeDark);
                theme.Items.Add(AppSettings.ThemeLight);
                AutomationProperties.SetName(theme, "Application theme");
                theme.SelectionChanged += delegate { if (!syncing && theme.SelectedItem != null) owner.themeCombo.SelectedItem = theme.SelectedItem; };
                Grid.SetColumn(theme, 1);
                themeRow.Children.Add(theme);
                themeCard.Children.Add(themeRow);
            }

            private void BuildAppearance()
            {
                Heading("Appearance", "Choose which details appear in the battery popup.");
                StackPanel colors = AddCard();
                colors.Children.Add(Text("Battery colors", 15, true, false));
                Space(colors, 8);
                colorByLevel = Check("Color by battery level", delegate(bool value) { owner.colorBarsCheckBox.Checked = value; });
                colors.Children.Add(colorByLevel);
                Space(colors, 5);
                colors.Children.Add(Text("Use the warning thresholds to color battery indicators.", 13, false, true));
                StackPanel popup = AddCard();
                popup.Children.Add(Text("Popup details", 15, true, false));
                Space(popup, 8);
                moreDevices = Check("Show more devices", delegate(bool value) { owner.showResetAvailabilityCheckBox.Checked = value; });
                details = Check("Show device details", delegate(bool value) { owner.showResetTimesCheckBox.Checked = value; });
                updated = Check("Show updated time", delegate(bool value) { owner.showLastUpdatedCheckBox.Checked = value; });
                popup.Children.Add(moreDevices); popup.Children.Add(details); popup.Children.Add(updated);
            }

            private RadioButton MetricOption(Panel panel, string title, string description)
            {
                RadioButton option = new RadioButton { GroupName = "TrayMetric", Margin = new Thickness(0, 0, 0, 5),
                    HorizontalContentAlignment = HorizontalAlignment.Stretch };
                StackPanel label = new StackPanel();
                label.Children.Add(Text(title, 16, true, false));
                label.Children.Add(Text(description, 13, false, true));
                option.Content = label;
                AutomationProperties.SetName(option, title);
                panel.Children.Add(option);
                return option;
            }

            private CheckBox Check(string label, Action<bool> update)
            {
                CheckBox box = new CheckBox { Content = label, Margin = new Thickness(0, 3, 0, 3) };
                AutomationProperties.SetName(box, label);
                box.Checked += delegate { if (!syncing) update(true); };
                box.Unchecked += delegate { if (!syncing) update(false); };
                return box;
            }

            private static WpfNumberBox NumberBox(double min, double max, string label)
            {
                WpfNumberBox box = new WpfNumberBox { Minimum = min, Maximum = max, SmallChange = 1, MaxDecimalPlaces = 0,
                    Width = 145, MinHeight = 30, Margin = new Thickness(0, 0, 4, 0) };
                AutomationProperties.SetName(box, label);
                return box;
            }

            private static WpfButton SmallButton(string text)
            {
                return new WpfButton { Content = text, MinHeight = 30, Padding = new Thickness(9, 3, 9, 3),
                    Margin = new Thickness(0, 0, 6, 4) };
            }

            private void AddSwatch(Panel parent, System.Drawing.Color value, bool isBackground, string label)
            {
                WpfButton button = SmallButton("");
                button.Width = 32; button.Height = 30; button.Padding = new Thickness(0);
                button.Background = Brush(value.R, value.G, value.B);
                button.BorderBrush = border;
                button.Tag = (isBackground ? "background:" : "text:") + value.ToArgb().ToString();
                AutomationProperties.SetName(button, label);
                button.Click += delegate
                {
                    if (isBackground) { owner.trayBoxColor = value; owner.useCustomTrayBoxColor = true; }
                    else owner.trayTextColor = value;
                    owner.UpdatePreviews();
                };
                swatches.Add(button);
                parent.Children.Add(button);
            }

            public void RefreshDevices()
            {
                if (device == null) return;
                bool old = syncing; syncing = true;
                device.Items.Clear();
                foreach (object item in owner.deviceCombo.Items) device.Items.Add(item);
                device.SelectedIndex = owner.deviceCombo.SelectedIndex;
                device.IsEnabled = owner.deviceRadio.Checked;
                UpdateDeviceDescription();
                syncing = old;
            }

            private void UpdateDeviceDescription()
            {
                if (device == null) return;
                string description = device.SelectedItem == null ? "" : device.SelectedItem.ToString();
                device.ToolTip = description;
                AutomationProperties.SetHelpText(device, description);
            }

            private void RefreshNumbers()
            {
                bool old = syncing; syncing = true;
                if (critical != null) critical.Value = (double)owner.criticalNumeric.Value;
                if (low != null) low.Value = (double)owner.lowNumeric.Value;
                syncing = old;
            }

            public void RefreshStatus()
            {
                if (updateStatus != null) updateStatus.Text = owner.updateStatusLabel.Text;
            }

            public void RefreshUpdateButtonState()
            {
                if (checkUpdates == null) return;
                checkUpdates.IsEnabled = owner.checkUpdatesButton.Enabled;
                checkUpdates.Content = owner.checkUpdatesButton.Text;
            }

            public void RefreshPreview()
            {
                if (preview == null) return;
                using (System.Drawing.Bitmap bitmapSource = IconRenderer.CreatePreviewBitmap(
                    (int)owner.previewPercentNumeric.Value, owner.CreatePreviewSettings(), 36))
                using (MemoryStream stream = new MemoryStream())
                {
                    bitmapSource.Save(stream, System.Drawing.Imaging.ImageFormat.Png);
                    stream.Position = 0;
                    BitmapImage bitmap = new BitmapImage();
                    bitmap.BeginInit(); bitmap.CacheOption = BitmapCacheOption.OnLoad; bitmap.StreamSource = stream; bitmap.EndInit();
                    bitmap.Freeze(); preview.Source = bitmap;
                }
                backgroundValue.Text = owner.trayBoxHexLabel.Text;
                textValue.Text = owner.trayTextHexLabel.Text;
                backgroundChoices.ToolTip = backgroundValue.Text;
                textChoices.ToolTip = textValue.Text;
                AutomationProperties.SetHelpText(backgroundChoices, "Current background: " + backgroundValue.Text);
                AutomationProperties.SetHelpText(textChoices, "Current text color: " + textValue.Text);
                contrast.Text = owner.contrastStatusLabel.Text;
                contrast.Foreground = !WinForms.SystemInformation.HighContrast && owner.contrastStatusLabel.ForeColor.ToArgb() == owner.GetWarningColor().ToArgb()
                    ? (AppSettings.IsDarkTheme(owner.GetSelectedTheme()) ? Brush(255, 205, 86) : Brush(170, 0, 0)) : secondary;
                autoColor.IsEnabled = true;
                autoColor.BorderBrush = owner.useCustomTrayBoxColor ? border : accent;
                autoColor.BorderThickness = new Thickness(owner.useCustomTrayBoxColor ? 1 : 2);
                AutomationProperties.SetHelpText(autoColor, owner.useCustomTrayBoxColor ? "Use automatic background color" : "Selected: automatic background color");
                foreach (WpfButton swatch in swatches)
                {
                    string tag = swatch.Tag as string;
                    System.Drawing.Color currentText = owner.trayTextColor;
                    System.Drawing.Color currentBox = owner.trayBoxColor;
                    bool selected = tag == "text:" + currentText.ToArgb().ToString() ||
                        (owner.useCustomTrayBoxColor && tag == "background:" + currentBox.ToArgb().ToString());
                    swatch.BorderBrush = selected ? accent : border;
                    swatch.BorderThickness = new Thickness(selected ? 2 : 1);
                    AutomationProperties.SetHelpText(swatch, selected ? "Selected" : "Select this color");
                }
            }

            public void RefreshFromOwner()
            {
                syncing = true;
                if (lowest != null) { lowest.IsChecked = owner.weeklyRadio.Checked; average.IsChecked = owner.fiveHourRadio.Checked; specific.IsChecked = owner.deviceRadio.Checked; }
                if (background != null) background.IsChecked = owner.showTrayBoxCheckBox.Checked;
                if (start != null) start.IsChecked = owner.startWithWindowsCheckBox.Checked;
                if (notifications != null) notifications.IsChecked = owner.thresholdNotificationsCheckBox.Checked;
                if (colorByLevel != null) colorByLevel.IsChecked = owner.colorBarsCheckBox.Checked;
                if (moreDevices != null) moreDevices.IsChecked = owner.showResetAvailabilityCheckBox.Checked;
                if (details != null) details.IsChecked = owner.showResetTimesCheckBox.Checked;
                if (updated != null) updated.IsChecked = owner.showLastUpdatedCheckBox.Checked;
                if (refresh != null) refresh.Value = (double)owner.refreshNumeric.Value;
                if (previewPercent != null) previewPercent.Value = (double)owner.previewPercentNumeric.Value;
                if (theme != null) theme.SelectedItem = owner.GetSelectedTheme();
                RefreshNumbers(); RefreshDevices(); RefreshStatus(); RefreshUpdateButtonState(); RefreshPreview();
                syncing = false;
            }
        }
    }
}
