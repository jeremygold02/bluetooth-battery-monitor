using System;
using System.Collections.Generic;
using System.Globalization;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Shapes;
using Wpf.Ui.Markup;
using SymbolIcon = Wpf.Ui.Controls.SymbolIcon;
using SymbolRegular = Wpf.Ui.Controls.SymbolRegular;

namespace CodexUsageTray
{
    // WPF lays out the text, indicators and buttons independently of their content.
    // Only the circular battery indicator is drawn as a vector shape.
    internal sealed class PopupView : UserControl
    {
        private const double MaximumListHeight = 300;
        private readonly Border frame;
        private readonly Grid content;
        private readonly StackPanel rows;
        private readonly ScrollViewer scroll;
        private readonly TextBlock metadata;
        private readonly TextBlock footer;
        private readonly Border logo;
        private readonly Wpf.Ui.Controls.Button refresh;
        private readonly Wpf.Ui.Controls.Button settingsButton;
        private AppSettings settings;
        private UsageSnapshot snapshot;
        private bool refreshing;
        private bool dark;
        private bool highContrast;
        private Brush foreground;
        private Brush muted;
        private Brush surface;
        private Brush border;
        private Brush accent;

        internal event EventHandler RefreshRequested;
        internal event EventHandler SettingsRequested;

        internal PopupView(AppSettings settings)
        {
            this.settings = settings;
            UseLayoutRounding = true;
            SnapsToDevicePixels = true;
            Resources.MergedDictionaries.Add(new ThemesDictionary());
            Resources.MergedDictionaries.Add(new ControlsDictionary());

            frame = new Border { CornerRadius = new CornerRadius(9), BorderThickness = new Thickness(1), Padding = new Thickness(12, 10, 12, 10) };
            content = new Grid();
            content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            content.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
            content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            frame.Child = content;
            Content = frame;

            Grid header = new Grid { Margin = new Thickness(0, 0, 0, 10), MinHeight = 36 };
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

            logo = new Border { Width = 34, Height = 34, CornerRadius = new CornerRadius(8), Margin = new Thickness(0, 0, 11, 0), VerticalAlignment = VerticalAlignment.Center };
            logo.Child = Icon(SymbolRegular.Bluetooth24, 21, Brushes.White);
            header.Children.Add(logo);
            WrapPanel headerText = new WrapPanel { VerticalAlignment = VerticalAlignment.Center };
            TextBlock title = new TextBlock { Text = "Bluetooth Battery", FontFamily = new FontFamily("Segoe UI"), FontSize = 16, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };
            title.SetResourceReference(TextBlock.ForegroundProperty, "PopupForeground");
            metadata = new TextBlock { FontFamily = new FontFamily("Segoe UI"), FontSize = 12.5, Margin = new Thickness(10, 2, 0, 0), VerticalAlignment = VerticalAlignment.Center, TextWrapping = TextWrapping.Wrap };
            metadata.SetResourceReference(TextBlock.ForegroundProperty, "PopupMuted");
            headerText.Children.Add(title);
            headerText.Children.Add(metadata);
            Grid.SetColumn(headerText, 1);
            header.Children.Add(headerText);
            refresh = HeaderButton(SymbolRegular.ArrowClockwise24, "Check batteries");
            refresh.Click += delegate { if (!IsRefreshing && RefreshRequested != null) RefreshRequested(this, EventArgs.Empty); };
            Grid.SetColumn(refresh, 2);
            header.Children.Add(refresh);
            settingsButton = HeaderButton(SymbolRegular.Settings24, "Open settings");
            settingsButton.Click += delegate { if (SettingsRequested != null) SettingsRequested(this, EventArgs.Empty); };
            Grid.SetColumn(settingsButton, 3);
            header.Children.Add(settingsButton);
            content.Children.Add(header);

            rows = new StackPanel();
            scroll = new ScrollViewer
            {
                Content = rows,
                HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                MaxHeight = MaximumListHeight,
                Focusable = true
            };
            AutomationProperties.SetName(scroll, "Wireless devices");
            Grid.SetRow(scroll, 1);
            content.Children.Add(scroll);

            footer = new TextBlock { FontFamily = new FontFamily("Segoe UI"), FontSize = 13, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 8, 0, 0), Visibility = Visibility.Collapsed };
            Grid.SetRow(footer, 2);
            content.Children.Add(footer);

            ApplySettings(settings);
        }

        internal int DeviceCount
        {
            get
            {
                int count = Devices().Count;
                return settings != null && !settings.ShowResetAvailability ? Math.Min(2, count) : count;
            }
        }
        internal ScrollViewer DeviceScroll { get { return scroll; } }
        internal Wpf.Ui.Controls.Button RefreshButton { get { return refresh; } }
        internal Wpf.Ui.Controls.Button SettingsButton { get { return settingsButton; } }
        internal bool IsRefreshing { get { return refreshing || (snapshot != null && snapshot.IsRefreshing); } }

        internal void ApplySettings(AppSettings value)
        {
            settings = value;
            highContrast = System.Windows.Forms.SystemInformation.HighContrast;
            dark = !highContrast && (settings == null || AppSettings.IsDarkTheme(settings.Theme));
            Color systemWindow = SystemColors.WindowColor;
            bool lightHighContrast = systemWindow.R + systemWindow.G + systemWindow.B >= 384;
            Resources.MergedDictionaries[0] = CreateThemeDictionary(dark, highContrast, lightHighContrast);
            foreground = highContrast ? SystemColors.WindowTextBrush : Brush(dark ? "#F2F3F5" : "#1C222A");
            muted = highContrast ? SystemColors.WindowTextBrush : Brush(dark ? "#C3C9D2" : "#505B68");
            surface = highContrast ? SystemColors.WindowBrush : Brush(dark ? "#26282C" : "#FFFFFF");
            border = highContrast ? SystemColors.WindowTextBrush : Brush(dark ? "#5C6067" : "#BEC7D0");
            accent = highContrast ? SystemColors.HighlightBrush : Brush(dark ? "#5AA9FF" : "#0069C8");
            Resources["PopupForeground"] = foreground;
            Resources["PopupMuted"] = muted;
            frame.Background = highContrast ? SystemColors.WindowBrush : Brush(dark ? "#161616" : "#F4F6F8");
            frame.BorderBrush = border;
            logo.Background = highContrast ? SystemColors.HighlightBrush : Brush(dark ? "#1F5790" : "#0069C8");
            Resources["PopupActionBackground"] = highContrast ? SystemColors.ControlBrush : Brush(dark ? "#30343A" : "#E5EBF1");
            Resources["PopupActionBorder"] = highContrast ? SystemColors.ControlTextBrush : Brush(dark ? "#5C6067" : "#AAB6C2");
            footer.Foreground = highContrast ? SystemColors.WindowTextBrush : Brush(dark ? "#FFD380" : "#7E4800");
            RebuildRows();
            UpdateHeader();
        }

        internal void UpdateSnapshot(UsageSnapshot value)
        {
            snapshot = value;
            RebuildRows();
            UpdateHeader();
        }

        internal void SetRefreshing(bool value)
        {
            bool wasRefreshing = IsRefreshing;
            refreshing = value;
            if (wasRefreshing != IsRefreshing && Devices().Count == 0) RebuildRows();
            UpdateHeader();
        }

        internal double MeasureHeight(double width, double maximumHeight)
        {
            Width = Math.Max(1, width);
            // Keep the header and status visible when the available work area is small.
            scroll.MaxHeight = Math.Max(1, Math.Min(MaximumListHeight, maximumHeight - 90));
            double contentWidth = Math.Max(1, width - 26);
            rows.Margin = new Thickness(0);
            rows.InvalidateMeasure();
            rows.Measure(new Size(contentWidth, double.PositiveInfinity));
            if (rows.DesiredSize.Height > scroll.MaxHeight)
            {
                // WPF UI's scrollbar overlays its content, so reserve room for its thumb.
                rows.Margin = new Thickness(0, 0, 12, 0);
                rows.InvalidateMeasure();
                rows.Measure(new Size(contentWidth, double.PositiveInfinity));
            }
            scroll.Height = Math.Min(scroll.MaxHeight, rows.DesiredSize.Height);
            scroll.InvalidateMeasure();
            scroll.Measure(new Size(contentWidth, scroll.Height));
            content.InvalidateMeasure();
            content.Measure(new Size(contentWidth, double.PositiveInfinity));
            frame.InvalidateMeasure();
            frame.Measure(new Size(width, double.PositiveInfinity));
            InvalidateMeasure();
            Measure(new Size(Width, double.PositiveInfinity));
            return Math.Max(1, Math.Min(maximumHeight, Math.Ceiling(frame.DesiredSize.Height)));
        }

        internal void ScrollBy(double amount) { scroll.ScrollToVerticalOffset(scroll.VerticalOffset + amount); }
        internal void ScrollToStart() { scroll.ScrollToTop(); }
        internal void ScrollToEnd() { scroll.ScrollToBottom(); }

        private void UpdateHeader()
        {
            int count = Devices().Count;
            metadata.Text = count.ToString(CultureInfo.CurrentCulture) + (count == 1 ? " device" : " devices");
            if ((settings == null || settings.ShowPopupLastUpdated) && snapshot != null && snapshot.LastUpdated != DateTime.MinValue)
                metadata.Text += "  |  Updated " + TimeFormatter.FormatClock(snapshot.LastUpdated);
            refresh.IsEnabled = !IsRefreshing;
            AutomationProperties.SetName(refresh, IsRefreshing ? "Checking batteries" : "Check batteries");
            refresh.ToolTip = IsRefreshing ? "Checking batteries" : "Check batteries";
            string status = StatusLine();
            footer.Text = status ?? "";
            footer.Visibility = string.IsNullOrEmpty(status) ? Visibility.Collapsed : Visibility.Visible;
        }

        private void RebuildRows()
        {
            double offset = scroll.VerticalOffset;
            List<LimitWindow> devices = Devices();
            rows.Children.Clear();
            int count = settings != null && !settings.ShowResetAvailability ? Math.Min(2, devices.Count) : devices.Count;
            for (int i = 0; i < count; i++) rows.Children.Add(CreateRow(devices[i]));
            if (count == 0)
            {
                TextBlock empty = new TextBlock { Text = snapshot == null || IsRefreshing ? "Checking devices..." : "No supported wireless battery devices found.", Foreground = muted, FontSize = 14, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 8, 0, 8) };
                rows.Children.Add(empty);
            }
            scroll.ScrollToVerticalOffset(offset);
        }

        private Border CreateRow(LimitWindow device)
        {
            Border card = new Border { Background = surface, CornerRadius = new CornerRadius(6), MinHeight = 60, Margin = new Thickness(0, 0, 0, 1), Focusable = true };
            Grid grid = new Grid { Margin = new Thickness(0, 5, 8, 5) };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(40) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(56) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(64) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            card.Child = grid;
            FrameworkElement deviceIcon = DeviceIcon(device.DisplayName, foreground);
            grid.Children.Add(deviceIcon);
            grid.Children.Add(Divider(1));
            StackPanel battery = new StackPanel { Orientation = Orientation.Horizontal,
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
            FrameworkElement gauge = Gauge(device);
            battery.Children.Add(gauge);
            TextBlock percent = new TextBlock { Text = device.BatteryPercent.HasValue ? device.RemainingPercent.ToString(CultureInfo.CurrentCulture) + "%" : "--", FontFamily = new FontFamily("Segoe UI"), FontSize = 22, FontWeight = FontWeights.SemiBold, Foreground = foreground, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0) };
            battery.Children.Add(percent);
            Grid.SetColumn(battery, 2);
            Grid.SetColumnSpan(battery, 2);
            grid.Children.Add(battery);
            grid.Children.Add(Divider(4));
            Grid information = new Grid { Margin = new Thickness(8, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
            information.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            information.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            string name = string.IsNullOrWhiteSpace(device.DisplayName) ? "Wireless device" : device.DisplayName;
            TextBlock nameText = new TextBlock { Text = name, FontFamily = new FontFamily("Segoe UI"), FontSize = 14.5, FontWeight = FontWeights.SemiBold, Foreground = foreground, TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
            information.Children.Add(nameText);
            string detail = DetailText(device);
            TextBlock detailText = new TextBlock { Text = detail ?? "", FontFamily = new FontFamily("Segoe UI"), FontSize = 13, Foreground = muted, TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 3, 0, 0), Visibility = string.IsNullOrEmpty(detail) ? Visibility.Collapsed : Visibility.Visible };
            Grid.SetRow(detailText, 1);
            information.Children.Add(detailText);
            Grid.SetColumn(information, 5);
            grid.Children.Add(information);
            List<string> description = new List<string>();
            description.Add(name);
            description.Add("Battery: " + (device.BatteryPercent.HasValue ? device.RemainingPercent.ToString(CultureInfo.CurrentCulture) + "%" : "unavailable"));
            if (!string.IsNullOrWhiteSpace(device.DetailLeft)) description.Add("Source: " + device.DetailLeft.Trim());
            if (!string.IsNullOrWhiteSpace(device.DetailRight)) description.Add("Status: " + device.DetailRight.Trim());
            if (device.IsCached && IsRoutineStatus(Compact(device.DetailRight, 200))) description.Add("Reading: Cached");
            if (!string.IsNullOrWhiteSpace(device.BatteryEstimate)) description.Add("Rate: " + device.BatteryEstimate.Trim());
            AutomationProperties.SetName(card, string.Join(", ", description.ToArray()));
            card.ToolTip = DeviceToolTip(name, description);
            ToolTipService.SetInitialShowDelay(card, 0);
            ToolTipService.SetBetweenShowDelay(card, 0);
            ToolTipService.SetShowDuration(card, 15000);
            return card;
        }

        private ToolTip DeviceToolTip(string name, List<string> description)
        {
            StackPanel content = new StackPanel { Width = 300 };
            content.Children.Add(new TextBlock { Text = name, FontFamily = new FontFamily("Segoe UI"), FontSize = 14,
                FontWeight = FontWeights.SemiBold, Foreground = foreground, TextWrapping = TextWrapping.Wrap,
                Margin = new Thickness(0, 0, 0, 8) });
            Grid details = new Grid();
            details.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            details.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            for (int i = 1; i < description.Count; i++)
            {
                int separator = description[i].IndexOf(": ", StringComparison.Ordinal);
                details.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                TextBlock label = new TextBlock { Text = description[i].Substring(0, separator), Foreground = muted,
                    FontSize = 12, Margin = new Thickness(0, 0, 14, 4) };
                TextBlock value = new TextBlock { Text = description[i].Substring(separator + 2), Foreground = foreground,
                    FontSize = 12, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 4) };
                Grid.SetRow(label, i - 1);
                Grid.SetRow(value, i - 1);
                Grid.SetColumn(value, 1);
                details.Children.Add(label);
                details.Children.Add(value);
            }
            content.Children.Add(details);
            return new ToolTip { Content = content, Background = surface, Foreground = foreground, BorderBrush = border,
                BorderThickness = new Thickness(1), Padding = new Thickness(12), FontFamily = new FontFamily("Segoe UI") };
        }

        private FrameworkElement Gauge(LimitWindow device)
        {
            const double center = 20;
            const double radius = 16;
            const double stroke = 4;
            Grid gauge = new Grid { Width = 40, Height = 40, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
            EllipseGeometry circle = new EllipseGeometry(new Point(center, center), radius, radius);
            System.Windows.Shapes.Path track = new System.Windows.Shapes.Path { Data = circle, Width = 40, Height = 40,
                Stroke = highContrast ? SystemColors.GrayTextBrush : Brush(dark ? "#555B64" : "#CAD3DD"), StrokeThickness = stroke };
            gauge.Children.Add(track);
            Brush color = BatteryBrush(device.RemainingPercent);
            if (device.BatteryPercent.HasValue && device.RemainingPercent > 0)
            {
                if (device.RemainingPercent >= 100)
                    gauge.Children.Add(new System.Windows.Shapes.Path { Data = circle, Width = 40, Height = 40, Stroke = color, StrokeThickness = stroke });
                else
                {
                    double angle = (device.RemainingPercent * 3.6 - 90) * Math.PI / 180.0;
                    Point end = new Point(center + radius * Math.Cos(angle), center + radius * Math.Sin(angle));
                    PathFigure figure = new PathFigure { StartPoint = new Point(center, center - radius), IsClosed = false, IsFilled = false };
                    figure.Segments.Add(new ArcSegment(end, new Size(radius, radius), 0, device.RemainingPercent > 50, SweepDirection.Clockwise, true));
                    PathGeometry geometry = new PathGeometry();
                    geometry.Figures.Add(figure);
                    gauge.Children.Add(new System.Windows.Shapes.Path { Data = geometry, Width = 40, Height = 40, Stroke = color, StrokeThickness = stroke, StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round });
                }
            }
            gauge.Children.Add(Icon(!device.BatteryPercent.HasValue ? SymbolRegular.BatteryWarning20 :
                (!string.IsNullOrEmpty(device.DetailRight) && device.DetailRight.StartsWith("Charging", StringComparison.OrdinalIgnoreCase) ? SymbolRegular.BatteryCharge20 : SymbolRegular.Battery1020), 18, color));
            return gauge;
        }

        private FrameworkElement Divider(int column)
        {
            Border line = new Border { Width = 1, Background = border, Margin = new Thickness(0, 5, 0, 5), Opacity = highContrast ? 1 : 0.65 };
            Grid.SetColumn(line, column);
            return line;
        }

        private Brush BatteryBrush(int percent)
        {
            if (highContrast) return SystemColors.HighlightBrush;
            if (settings != null && !settings.ColorBars) return Brush(dark ? "#A2A9B1" : "#68727D");
            if (settings != null && percent <= settings.LowThreshold)
            {
                System.Drawing.Color color = IconRenderer.ColorForPercent(percent, settings);
                return new SolidColorBrush(Color.FromArgb(color.A, color.R, color.G, color.B));
            }
            return accent;
        }

        private List<LimitWindow> Devices()
        {
            List<LimitWindow> result = new List<LimitWindow>();
            if (snapshot == null) return result;
            if (snapshot.Devices != null && snapshot.Devices.Count > 0)
            {
                foreach (LimitWindow device in snapshot.Devices) if (device != null) result.Add(device);
            }
            else
            {
                if (snapshot.Weekly != null) result.Add(snapshot.Weekly);
                if (snapshot.FiveHour != null) result.Add(snapshot.FiveHour);
            }
            return result;
        }

        private string StatusLine()
        {
            if (snapshot == null || IsRefreshing) return null;
            string status = Compact(snapshot.StatusMessage, 120);
            if (!string.IsNullOrEmpty(status)) return status;
            if (!string.IsNullOrEmpty(snapshot.ErrorMessage)) return Compact(snapshot.ErrorMessage, 120);
            if (snapshot.IsPaused) return "Automatic checks paused";
            return snapshot.IsStale ? "Battery levels may be stale" : null;
        }

        private string DetailText(LimitWindow device)
        {
            string status = Compact(device.DetailRight, 200);
            if (settings != null && !settings.ShowPopupResetTimes)
                return device.IsCached ? (IsRoutineStatus(status) ? "Cached reading" : status) : null;
            string estimate = Compact(device.BatteryEstimate, 200);
            string source = Compact(device.DetailLeft, 200);
            List<string> details = new List<string>();
            if (!string.IsNullOrEmpty(source)) details.Add(source);
            if (!string.IsNullOrEmpty(estimate)) details.Add(estimate);
            if (!IsRoutineStatus(status)) details.Add(status);
            else if (device.IsCached) details.Add("Cached reading");
            return details.Count == 0 ? null : string.Join(" · ", details.ToArray());
        }

        private static bool IsRoutineStatus(string status)
        {
            return string.IsNullOrEmpty(status) || string.Equals(status, "Connected", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(status, "Not charging", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(status, "Discharging", StringComparison.OrdinalIgnoreCase);
        }

        private static string Compact(string value, int maximum)
        {
            if (string.IsNullOrWhiteSpace(value)) return null;
            string result = value.Replace('\r', ' ').Replace('\n', ' ').Replace('\t', ' ').Trim();
            while (result.IndexOf("  ", StringComparison.Ordinal) >= 0) result = result.Replace("  ", " ");
            return result.Length > maximum ? result.Substring(0, maximum - 3).TrimEnd() + "..." : result;
        }

        private static Wpf.Ui.Controls.Button HeaderButton(SymbolRegular symbol, string description)
        {
            Wpf.Ui.Controls.Button button = new Wpf.Ui.Controls.Button { Width = 34, Height = 34, Padding = new Thickness(0), Margin = new Thickness(4, 0, 0, 0), CornerRadius = new CornerRadius(7), Icon = new SymbolIcon(symbol, 20, false), ToolTip = description, VerticalAlignment = VerticalAlignment.Center };
            button.SetResourceReference(Control.BackgroundProperty, "PopupActionBackground");
            button.SetResourceReference(Control.BorderBrushProperty, "PopupActionBorder");
            button.SetResourceReference(Control.ForegroundProperty, "PopupForeground");
            AutomationProperties.SetName(button, description);
            ToolTipService.SetInitialShowDelay(button, 0);
            ToolTipService.SetBetweenShowDelay(button, 0);
            return button;
        }

        private static SymbolIcon Icon(SymbolRegular symbol, double size, Brush color)
        {
            return new SymbolIcon(symbol, size, false) { Foreground = color, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        }

        private static FrameworkElement DeviceIcon(string name, Brush color)
        {
            string value = (name ?? "").ToLowerInvariant();
            FrameworkElement icon;
            string kind;
            if (value.Contains("mouse") || value.Contains("superlight") || value.Contains("g305") || value.Contains("pulsefire")) { icon = FontIcon("Segoe MDL2 Assets", "\uE962", color); kind = "mouse"; }
            else if (value.Contains("earbud") || value.Contains("airpod") || value.Contains("buds"))
            {
                GlyphTypeface typeface;
                bool hasEarbudGlyph = new Typeface("Segoe MDL2 Assets").TryGetGlyphTypeface(out typeface) &&
                    typeface.CharacterToGlyphMap.ContainsKey(0xF4C0);
                icon = hasEarbudGlyph ? (FrameworkElement)FontIcon("Segoe MDL2 Assets", "\uF4C0", color) : Icon(SymbolRegular.Headphones24, 25, color);
                kind = "earbuds";
            }
            else if (value.Contains("headset") || value.Contains("hyperx")) { icon = Icon(SymbolRegular.Headset24, 25, color); kind = "headset"; }
            else if (value.Contains("headphone")) { icon = Icon(SymbolRegular.Headphones24, 25, color); kind = "headphones"; }
            else if (value.Contains("controller") || value.Contains("gamepad") || value.Contains("xbox")) { icon = Icon(SymbolRegular.Games24, 25, color); kind = "controller"; }
            else if (value.Contains("keyboard")) { icon = Icon(SymbolRegular.Keyboard24, 25, color); kind = "keyboard"; }
            else { icon = Icon(SymbolRegular.Bluetooth24, 25, color); kind = "wireless device"; }
            AutomationProperties.SetName(icon, kind);
            return icon;
        }

        private static TextBlock FontIcon(string family, string glyph, Brush color)
        {
            return new TextBlock { Text = glyph, FontFamily = new FontFamily(family), FontSize = 25, Foreground = color, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        }

        private static Brush Brush(string value) { return new SolidColorBrush((Color)ColorConverter.ConvertFromString(value)); }

        internal static ThemesDictionary CreateThemeDictionary(bool useDark, bool useHighContrast, bool lightHighContrast)
        {
            ThemesDictionary theme = new ThemesDictionary();
            if (useHighContrast)
                theme.Source = new Uri("pack://application:,,,/Wpf.Ui;component/Resources/Theme/" +
                    (lightHighContrast ? "HCWhite.xaml" : "HCBlack.xaml"), UriKind.Absolute);
            else
                theme.Theme = useDark ? Wpf.Ui.Appearance.ApplicationTheme.Dark : Wpf.Ui.Appearance.ApplicationTheme.Light;
            return theme;
        }
    }
}
