using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using CodexUsageTray;

internal static class PopupUiTests
{
    [STAThread]
    private static void Main(string[] args)
    {
        AppSettings settings = new AppSettings { Theme = AppSettings.ThemeDark };
        PopupView view = new PopupView(settings);
        UsageSnapshot snapshot = CreateSnapshot(4);
        view.UpdateSnapshot(snapshot);
        double fourHeight = Arrange(view, 480, 900);
        Check(view.DeviceCount == 4, "four devices are laid out");
        Check(fourHeight <= 370, "four-device popup stays compact");
        for (int i = 0; i < 4; i++) Check(Row(view, i).ActualHeight >= 60,
            "device row keeps its minimum height and can grow for details");
        Check(view.DeviceScroll.ComputedHorizontalScrollBarVisibility == Visibility.Collapsed, "device list has no horizontal scrollbar");
        AssertRowsDoNotOverlap(view, 4);
        AssertBottomPadding(view, 3);
        AssertHeaderAlignment(view);
        Check(!HasText(view, "Connected") && !HasText(view, "Charging"), "status pills are not rendered");
        Check(AutomationProperties.GetName(Row(view, 1)).Contains("Charging"), "charging remains available to accessibility tools");
        Check(DeviceIconName(view, 1) == "mouse" && DeviceIconName(view, 2) == "earbuds", "mouse and earbuds use the appropriate device symbols");
        Check(((TextBlock)((Grid)Row(view, 1).Child).Children[0]).Text == "\uE962", "mouse uses the mouse glyph");
        FrameworkElement hyperxMouseIcon = (FrameworkElement)typeof(PopupView).GetMethod("DeviceIcon", BindingFlags.Static | BindingFlags.NonPublic)
            .Invoke(null, new object[] { "HyperX Pulsefire mouse", Brushes.White });
        Check(AutomationProperties.GetName(hyperxMouseIcon) == "mouse", "HyperX mouse is not misidentified as a headset");
        Check(HasText(view, "Wireless · Est. -3.2%/h · ~14h 30m remaining"), "source and rate estimate are both visible");
        if (args.Length > 0) Capture(view, args[0], 1);

        snapshot = CreateSnapshot(2);
        snapshot.Devices[0].DetailLeft = "HyperX 2.4 GHz";
        snapshot.Devices[0].DetailRight = "Not charging";
        snapshot.Devices[0].BatteryEstimate = "Learning battery rate";
        snapshot.Devices[1].DetailLeft = "Logitech LIGHTSPEED";
        snapshot.Devices[1].DetailRight = "dIsChArGiNg";
        snapshot.Devices[1].BatteryEstimate = "Learning battery rate";
        view.UpdateSnapshot(snapshot);
        double twoHeight = Arrange(view, 480, 900);
        Check(twoHeight < fourHeight - 100, "popup shrinks when the device count drops from four to two");
        Check(twoHeight <= 230, "two-device popup remains compact");
        Check(!HasText(view, "Not charging") && !HasText(view, "dIsChArGiNg"), "passive status badges are hidden case-insensitively");
        Check(AutomationProperties.GetName(Row(view, 0)).Contains("Not charging"), "hidden status remains accessible");
        Check(AutomationProperties.GetName(Row(view, 1)).Contains("Logitech LIGHTSPEED"), "connection source remains accessible");
        Check(HasText(view, "HyperX 2.4 GHz · Learning battery rate"), "device source and learning rate are both shown");
        Check(view.DeviceScroll.ScrollableHeight == 0 && Bounds(Row(view, 1), view).Bottom <= Bounds(view.DeviceScroll, view).Bottom + 0.5,
            "both devices and their details fit without scrolling");
        AssertBottomPadding(view, 1);
        ToolTip hover = (ToolTip)Row(view, 0).ToolTip;
        FrameworkElement hoverContent = ArrangeToolTip(hover);
        Check(ToolTipService.GetInitialShowDelay(Row(view, 0)) == 0 &&
            ToolTipService.GetInitialShowDelay(view.RefreshButton) == 0 &&
            ToolTipService.GetInitialShowDelay(view.SettingsButton) == 0, "tooltips appear immediately");
        Check(HasText(hoverContent, "HyperX Cloud III S Wireless") && HasText(hoverContent, "Battery") && HasText(hoverContent, "47%") &&
            HasText(hoverContent, "Source") && HasText(hoverContent, "HyperX 2.4 GHz") && HasText(hoverContent, "Status") &&
            HasText(hoverContent, "Not charging") && HasText(hoverContent, "Rate") && HasText(hoverContent, "Learning battery rate"),
            "hover displays the device name and labeled details");
        if (args.Length > 0) SaveBitmap(RenderVisual(hover, hover.ActualWidth, hover.ActualHeight, 1), Variant(args[0], "-tooltip"));
        AssertBorderPixels(view);
        AssertRowsDoNotOverlap(view, 2);
        if (args.Length > 0) Capture(view, Variant(args[0], "-two"), 1);

        snapshot.StatusMessage = "Some device types could not be checked.";
        view.UpdateSnapshot(snapshot);
        Arrange(view, 480, 900);
        TextBlock footer = (TextBlock)Field(view, "footer");
        Rect footerBounds = Bounds(footer, view);
        Check(Math.Abs(footerBounds.Left - Bounds(Row(view, 0), view).Left) <= 1,
            "status message aligns with the device list");
        Check(footerBounds.Top - Bounds(view.DeviceScroll, view).Bottom >= 7.5 &&
            view.ActualHeight - footerBounds.Bottom >= 10,
            "status message has space above and below");
        if (args.Length > 0) Capture(view, Variant(args[0], "-status"), 1);
        snapshot.StatusMessage = null;

        snapshot.Devices[0].BatteryPercent = 100;
        snapshot.Devices[0].DetailRight = "Charging";
        view.UpdateSnapshot(snapshot);
        Arrange(view, 480, 900);
        TextBlock full = FindText(view, "100%");
        Check(full != null && full.DesiredSize.Width - full.Margin.Left - full.Margin.Right <= full.ActualWidth + 0.5,
            "100 percent is not clipped");
        Check(!HasText(view, "Charging"), "charging status does not render a pill");
        Check(((Wpf.Ui.Controls.SymbolIcon)Gauge(view, 0).Children[2]).Symbol == Wpf.Ui.Controls.SymbolRegular.BatteryCharge20,
            "charging retains its battery symbol");
        AssertRowsDoNotOverlap(view, 2);
        if (args.Length > 0) Capture(view, Variant(args[0], "-100"), 1);

        snapshot.Devices[0].IsCached = true;
        snapshot.Devices[0].DetailRight = "Last reported 1:23 PM · sleeping or unavailable";
        snapshot.Devices[0].BatteryEstimate = null;
        view.UpdateSnapshot(snapshot);
        Arrange(view, 480, 900);
        Check(HasText(view, "HyperX 2.4 GHz · Last reported 1:23 PM · sleeping or unavailable"),
            "cached battery readings are labeled in the popup");
        hoverContent = ArrangeToolTip((ToolTip)Row(view, 0).ToolTip);
        Check(HasText(hoverContent, "Last reported 1:23 PM · sleeping or unavailable"),
            "tooltip preserves the cached reading's status");
        settings.ShowPopupResetTimes = false;
        view.ApplySettings(settings);
        Arrange(view, 480, 900);
        Check(HasText(view, "Last reported 1:23 PM · sleeping or unavailable"),
            "cached readings remain labeled when optional details are hidden");
        snapshot.Devices[0].DetailRight = "Connected";
        view.UpdateSnapshot(snapshot);
        Arrange(view, 480, 900);
        Check(HasText(view, "Cached reading") && AutomationProperties.GetName(Row(view, 0)).Contains("Reading: Cached"),
            "a cached reading is identified even if its status is generic");
        hoverContent = ArrangeToolTip((ToolTip)Row(view, 0).ToolTip);
        Check(HasText(hoverContent, "Reading") && HasText(hoverContent, "Cached"), "generic cached readings are identified in the tooltip");
        snapshot.Devices[0].BatteryPercent = null;
        snapshot.Devices[0].DetailLeft = null;
        snapshot.Devices[0].DetailRight = null;
        view.UpdateSnapshot(snapshot);
        hoverContent = ArrangeToolTip((ToolTip)Row(view, 0).ToolTip);
        Check(HasText(hoverContent, "unavailable") && !HasText(hoverContent, "Source") && !HasText(hoverContent, "Rate"),
            "unknown battery and absent optional details are represented accurately");
        settings.ShowPopupResetTimes = true;
        view.ApplySettings(settings);

        snapshot = CreateSnapshot(8);
        snapshot.Devices[1].DisplayName = "A very long wireless mouse name that must wrap fully onto several readable lines without clipping";
        snapshot.Devices[1].DetailRight = "Last reported recently but the wireless device is currently unavailable";
        view.UpdateSnapshot(snapshot);
        Arrange(view, 480, 900);
        Check(Row(view, 1).ActualHeight > Row(view, 0).ActualHeight, "long names and status grow the row");
        hoverContent = ArrangeToolTip((ToolTip)Row(view, 1).ToolTip);
        TextBlock longName = FindText(hoverContent, snapshot.Devices[1].DisplayName);
        TextBlock longStatus = FindText(hoverContent, snapshot.Devices[1].DetailRight);
        Check(longName != null && longStatus != null && longName.TextWrapping == TextWrapping.Wrap &&
            longStatus.TextWrapping == TextWrapping.Wrap && longName.ActualHeight > longName.FontSize * 1.8 &&
            longStatus.ActualHeight > longStatus.FontSize * 1.8, "long tooltip names and values wrap without clipping");
        Check(view.DeviceScroll.ScrollableHeight > 0, "long lists remain vertically scrollable");
        Check(view.DeviceScroll.ComputedHorizontalScrollBarVisibility == Visibility.Collapsed, "long lists never require horizontal scrolling");
        Rect scrollBounds = Bounds(view.DeviceScroll, view);
        Grid firstInformation = (Grid)((Grid)Row(view, 0).Child).Children.Cast<UIElement>().First(child => Grid.GetColumn(child) == 5);
        Rect informationBounds = Bounds(firstInformation, view);
        Check(informationBounds.Right <= scrollBounds.Right - 8, "scroll thumb does not cover device information");
        if (args.Length > 0) Capture(view, Variant(args[0], "-long"), 1);
        view.ScrollToEnd();
        view.UpdateLayout();
        Check(view.DeviceScroll.VerticalOffset > 0, "scroll to end moves the list");
        view.UpdateSnapshot(snapshot);
        Arrange(view, 480, 900);
        Check(view.DeviceScroll.VerticalOffset > 0, "updating battery readings keeps the list position");
        view.ScrollToStart();
        view.UpdateLayout();
        Check(view.DeviceScroll.VerticalOffset == 0, "scroll to start returns to the first device");
        settings.ShowResetAvailability = false;
        view.ApplySettings(settings);
        Arrange(view, 480, 900);
        Check(view.DeviceCount == 2, "show-more setting can limit the list");
        settings.ShowResetAvailability = true;
        settings.ShowPopupResetTimes = false;
        view.ApplySettings(settings);
        Arrange(view, 480, 900);
        Grid noDetails = (Grid)((Grid)Row(view, 0).Child).Children.Cast<UIElement>().First(child => Grid.GetColumn(child) == 5);
        TextBlock nameOnly = (TextBlock)noDetails.Children[0];
        TextBlock hiddenDetail = (TextBlock)noDetails.Children[1];
        Check(hiddenDetail.Visibility == Visibility.Collapsed &&
            Math.Abs((Bounds(nameOnly, (Grid)Row(view, 0).Child).Top + Bounds(nameOnly, (Grid)Row(view, 0).Child).Bottom) / 2 -
                ((Grid)Row(view, 0).Child).ActualHeight / 2) <= 1,
            "device name stays vertically centered when details are hidden");
        Arrange(view, 304, 244);
        Check(view.DeviceCount == 8 && view.DeviceScroll.ComputedHorizontalScrollBarVisibility == Visibility.Collapsed,
            "small work areas preserve all rows and disable horizontal scrolling");
        if (args.Length > 0) Capture(view, Variant(args[0], "-small"), 1);
        view.ScrollToEnd();
        view.UpdateLayout();
        Check(view.DeviceScroll.VerticalOffset > 0 && Bounds(Row(view, 7), view).Bottom <= Bounds(view.DeviceScroll, view).Bottom + 1,
            "the final device is reachable in a small work area");

        int refreshEvents = 0, settingsEvents = 0;
        view.RefreshRequested += delegate { refreshEvents++; };
        view.SettingsRequested += delegate { settingsEvents++; };
        view.RefreshButton.RaiseEvent(new RoutedEventArgs(ButtonBase.ClickEvent));
        view.SettingsButton.RaiseEvent(new RoutedEventArgs(ButtonBase.ClickEvent));
        Check(refreshEvents == 1 && settingsEvents == 1, "header actions raise the expected events");
        view.SetRefreshing(true);
        Check(!view.RefreshButton.IsEnabled, "refresh action is disabled while checking");
        view.RefreshButton.RaiseEvent(new RoutedEventArgs(ButtonBase.ClickEvent));
        Check(refreshEvents == 1, "refresh cannot be requested while checking");
        view.SetRefreshing(false);
        Check(view.RefreshButton.IsEnabled, "refresh action is restored after checking");

        view.UpdateSnapshot(new UsageSnapshot());
        Arrange(view, 480, 900);
        Check(HasText(view, "No supported wireless battery devices found."), "empty result is shown");
        TextBlock empty = FindText(view, "No supported wireless battery devices found.");
        Border emptyFrame = (Border)Field(view, "frame");
        Check(Math.Abs(Bounds(empty, view).Left - (emptyFrame.BorderThickness.Left + emptyFrame.Padding.Left)) <= 1,
            "empty message aligns with the popup content");
        if (args.Length > 0) Capture(view, Variant(args[0], "-empty"), 1);
        view.SetRefreshing(true);
        Arrange(view, 480, 900);
        Check(HasText(view, "Checking devices..."), "refreshing an empty list shows the checking state");
        if (args.Length > 0) Capture(view, Variant(args[0], "-loading"), 1);
        view.SetRefreshing(false);
        Arrange(view, 480, 900);
        Check(HasText(view, "No supported wireless battery devices found."), "empty result is restored after checking");

        settings.Theme = AppSettings.ThemeLight;
        view.ApplySettings(settings);
        Arrange(view, 480, 900);
        Border frame = (Border)Field(view, "frame");
        Check(((SolidColorBrush)frame.Background).Color.R > 200, "light theme updates the popup background");
        if (args.Length > 0) Capture(view, Variant(args[0], "-light"), 1);
        view.UpdateSnapshot(CreateSnapshot(2));
        ToolTip lightTooltip = (ToolTip)Row(view, 0).ToolTip;
        ArrangeToolTip(lightTooltip);
        Check(((SolidColorBrush)lightTooltip.Background).Color.R > 200 && ((SolidColorBrush)lightTooltip.Foreground).Color.R < 100,
            "tooltip colors follow the light theme");
        if (args.Length > 0) SaveBitmap(RenderVisual(lightTooltip, lightTooltip.ActualWidth, lightTooltip.ActualHeight, 1), Variant(args[0], "-tooltip-light"));
        Check(PopupView.CreateThemeDictionary(false, true, false).Source.OriginalString.EndsWith("HCBlack.xaml", StringComparison.Ordinal),
            "dark high contrast theme resource loads");
        Check(PopupView.CreateThemeDictionary(false, true, true).Source.OriginalString.EndsWith("HCWhite.xaml", StringComparison.Ordinal),
            "light high contrast theme resource loads");

        // Rendering at 150% tests vector layout and output resolution without changing desktop DPI.
        settings.Theme = AppSettings.ThemeDark;
        settings.ShowPopupResetTimes = true;
        view.ApplySettings(settings);
        view.UpdateSnapshot(CreateSnapshot(2));
        Arrange(view, 480, 900);
        if (args.Length > 0) Capture(view, Variant(args[0], "-150"), 1.5);

        TestGaugeAlignment(view, settings, args.Length > 0 ? args[0] : null);

        using (UsagePopup popup = new UsagePopup(settings))
        {
            popup.UpdateSnapshot(CreateSnapshot(2));
            Check(popup.Width <= 480 && popup.Height <= 230, "WinForms host sizes to the two-device WPF view at 100% DPI");
            foreach (double scale in new double[] { 1, 1.25, 1.5, 1.75, 2 })
            {
                typeof(UsagePopup).GetField("dpiScale", BindingFlags.Instance | BindingFlags.NonPublic).SetValue(popup, scale);
                typeof(UsagePopup).GetMethod("UpdateRegion", BindingFlags.Instance | BindingFlags.NonPublic).Invoke(popup, null);
                Check(popup.Region.IsVisible(0, popup.Height / 2) && popup.Region.IsVisible(popup.Width - 1, popup.Height / 2) &&
                    popup.Region.IsVisible(popup.Width / 2, 0) && popup.Region.IsVisible(popup.Width / 2, popup.Height - 1),
                    "rounded window region includes all four border edges at scale " + scale);
                Check(!popup.Region.IsVisible(0, 0) && !popup.Region.IsVisible(popup.Width - 1, popup.Height - 1),
                    "popup retains rounded corners at scale " + scale);
            }
        }
        Console.WriteLine("Popup UI headless checks passed.");
    }

    private static double Arrange(PopupView view, double width, double maximumHeight)
    {
        double height = view.MeasureHeight(width, maximumHeight);
        view.Arrange(new Rect(0, 0, width, height));
        view.UpdateLayout();
        return height;
    }

    private static void AssertRowsDoNotOverlap(PopupView view, int count)
    {
        for (int i = 0; i < count; i++)
        {
            Border row = Row(view, i);
            Grid grid = (Grid)row.Child;
            FrameworkElement icon = (FrameworkElement)grid.Children[0];
            FrameworkElement firstDivider = (FrameworkElement)grid.Children[1];
            StackPanel battery = (StackPanel)grid.Children[2];
            FrameworkElement gauge = (FrameworkElement)battery.Children[0];
            FrameworkElement percent = (FrameworkElement)battery.Children[1];
            FrameworkElement secondDivider = (FrameworkElement)grid.Children[3];
            Rect iconBounds = Bounds(icon, grid);
            Rect firstDividerBounds = Bounds(firstDivider, grid);
            Rect gaugeBounds = Bounds(gauge, grid);
            Rect percentBounds = Bounds(percent, grid);
            Rect secondDividerBounds = Bounds(secondDivider, grid);
            Rect iconInRow = Bounds(icon, row);
            Rect firstDividerInRow = Bounds(firstDivider, row);
            Check(Math.Abs(iconInRow.Left - (firstDividerInRow.Left - iconInRow.Right)) <= 1,
                "device icon has balanced padding between the card edge and first divider on row " + i);
            Check(gaugeBounds.Left - firstDividerBounds.Right >= 7.5, "gauge has left padding on row " + i);
            Check(percentBounds.Left - gaugeBounds.Right >= 7.5 && percentBounds.Left - gaugeBounds.Right <= 8.5,
                "percentage has consistent spacing after the gauge on row " + i);
            Check(Math.Abs((gaugeBounds.Left - firstDividerBounds.Right) - (secondDividerBounds.Left - percentBounds.Right)) <= 1.5,
                "battery group has balanced padding between its dividers on row " + i);
            Check(percentBounds.Right <= secondDividerBounds.Left + 0.5, "percentage does not overlap divider on row " + i);
            Check(Math.Abs((gaugeBounds.Top + gaugeBounds.Bottom) / 2 - grid.ActualHeight / 2) < 1,
                "gauge is vertically centered on row " + i);
            Rect gaugeInRow = Bounds(gauge, row);
            Check(gaugeInRow.Top >= 8 && row.ActualHeight - gaugeInRow.Bottom >= 8 &&
                Math.Abs(gaugeInRow.Top - (row.ActualHeight - gaugeInRow.Bottom)) <= 1,
                "gauge has balanced vertical padding on row " + i + ": " + gaugeInRow + " in " + row.ActualHeight);
            Check(Math.Abs((iconBounds.Top + iconBounds.Bottom) / 2 - grid.ActualHeight / 2) < 1,
                "device icon is vertically centered on row " + i);
            Grid information = (Grid)grid.Children.Cast<UIElement>().First(child => Grid.GetColumn(child) == 5);
            TextBlock name = (TextBlock)information.Children[0];
            TextBlock detail = (TextBlock)information.Children[1];
            Rect nameBounds = Bounds(name, information);
            Rect detailBounds = Bounds(detail, information);
            Check(nameBounds.Bottom <= detailBounds.Top + 0.5, "name and detail do not overlap on row " + i);
            Check(Math.Abs(nameBounds.Right - detailBounds.Right) <= 1, "name and detail share the same right padding on row " + i);
            Check(information.Children.Count == 2, "row " + i + " contains no status badge");
            Check(grid.ActualWidth <= row.ActualWidth + 0.5, "row content stays inside its card " + i);
            Check(information.ActualHeight <= row.ActualHeight + 0.5, "row text stays inside its card " + i);
        }
    }

    private static Rect Bounds(FrameworkElement value, Visual ancestor)
    {
        return value.TransformToAncestor(ancestor).TransformBounds(new Rect(0, 0, value.ActualWidth, value.ActualHeight));
    }

    private static void AssertBottomPadding(PopupView view, int lastRow)
    {
        Check(view.DeviceScroll.ScrollableHeight == 0, "bottom padding is checked with all rows visible");
        double gap = view.ActualHeight - Bounds(Row(view, lastRow), view).Bottom;
        Check(gap >= 10 && gap <= 13, "last device has consistent bottom padding: " + gap);
    }

    private static void AssertHeaderAlignment(PopupView view)
    {
        Rect card = Bounds(Row(view, 0), view);
        Rect logo = Bounds((FrameworkElement)Field(view, "logo"), view);
        Rect refresh = Bounds(view.RefreshButton, view);
        Rect settings = Bounds(view.SettingsButton, view);
        Check(Math.Abs(logo.Left - card.Left) <= 1 && Math.Abs(settings.Right - card.Right) <= 1,
            "header and device cards share the same outer gutters");
        Check(Math.Abs((logo.Top + logo.Bottom) / 2 - (refresh.Top + refresh.Bottom) / 2) <= 1 &&
            Math.Abs((refresh.Top + refresh.Bottom) / 2 - (settings.Top + settings.Bottom) / 2) <= 1,
            "header logo and action buttons are vertically aligned");
    }

    private static void AssertBorderPixels(PopupView view)
    {
        RenderTargetBitmap bitmap = RenderVisual(view, view.ActualWidth, view.ActualHeight, 1);
        int midX = bitmap.PixelWidth / 2, midY = bitmap.PixelHeight / 2;
        byte[] top = Pixel(bitmap, midX, 0);
        Check(top[3] == 255 && top.SequenceEqual(Pixel(bitmap, midX, bitmap.PixelHeight - 1)) &&
            top.SequenceEqual(Pixel(bitmap, 0, midY)) && top.SequenceEqual(Pixel(bitmap, bitmap.PixelWidth - 1, midY)),
            "WPF border renders consistently on all four straight edges");
    }

    private static byte[] Pixel(BitmapSource bitmap, int x, int y)
    {
        byte[] value = new byte[4];
        bitmap.CopyPixels(new Int32Rect(x, y, 1, 1), value, 4, 0);
        return value;
    }

    private static Border Row(PopupView view, int index)
    {
        return (Border)((StackPanel)Field(view, "rows")).Children[index];
    }

    private static Grid Gauge(PopupView view, int index)
    {
        return (Grid)((StackPanel)((Grid)Row(view, index).Child).Children[2]).Children[0];
    }

    private static void TestGaugeAlignment(PopupView view, AppSettings settings, string screenshotPath)
    {
        int originalLowThreshold = settings.LowThreshold;
        settings.LowThreshold = 0;
        view.ApplySettings(settings);
        foreach (int percent in new int[] { 1, 25, 50, 75, 99, 100 })
        {
            UsageSnapshot value = CreateSnapshot(2);
            value.Devices[0].BatteryPercent = percent;
            value.Devices[0].DetailRight = "Charging";
            view.UpdateSnapshot(value);
            Arrange(view, 480, 900);
            Grid gauge = Gauge(view, 0);
            System.Windows.Shapes.Path track = (System.Windows.Shapes.Path)gauge.Children[0];
            System.Windows.Shapes.Path fill = (System.Windows.Shapes.Path)gauge.Children[1];
            EllipseGeometry circle = (EllipseGeometry)track.Data;
            Check(circle.Center == new Point(20, 20) && circle.RadiusX == 16 && circle.RadiusY == 16,
                "track uses the expected center and radius at " + percent + "%");
            Check(track.StrokeThickness == fill.StrokeThickness, "track and arc stroke thickness match at " + percent + "%");
            if (percent == 100)
            {
                EllipseGeometry full = (EllipseGeometry)fill.Data;
                Check(full.Center == circle.Center && full.RadiusX == circle.RadiusX && full.RadiusY == circle.RadiusY,
                    "full ring shares its track geometry");
            }
            else
            {
                PathFigure figure = ((PathGeometry)fill.Data).Figures[0];
                ArcSegment arc = (ArcSegment)figure.Segments[0];
                double radians = (percent * 3.6 - 90) * Math.PI / 180;
                Check(figure.StartPoint == new Point(circle.Center.X, circle.Center.Y - circle.RadiusY) &&
                    arc.Size.Width == circle.RadiusX && arc.Size.Height == circle.RadiusY &&
                    Math.Abs(arc.Point.X - (circle.Center.X + circle.RadiusX * Math.Cos(radians))) < 0.001 &&
                    Math.Abs(arc.Point.Y - (circle.Center.Y + circle.RadiusY * Math.Sin(radians))) < 0.001,
                    "arc shares the track center and radius at " + percent + "%");
            }
            Check(((Wpf.Ui.Controls.SymbolIcon)gauge.Children[2]).Symbol == Wpf.Ui.Controls.SymbolRegular.BatteryCharge20,
                "charging symbol is retained at " + percent + "%");
            foreach (double scale in new double[] { 1, 1.5 })
            {
                RenderTargetBitmap whole = RenderVisual(view, view.ActualWidth, view.ActualHeight, scale);
                Rect gaugeBounds = Bounds(gauge, view);
                BitmapSource bitmap = new CroppedBitmap(whole, new Int32Rect(
                    (int)Math.Round(gaugeBounds.X * scale), (int)Math.Round(gaugeBounds.Y * scale),
                    (int)Math.Round(gaugeBounds.Width * scale), (int)Math.Round(gaugeBounds.Height * scale)));
                Color trackColor = ((SolidColorBrush)track.Stroke).Color;
                Color fillColor = ((SolidColorBrush)fill.Stroke).Color;
                if (screenshotPath != null)
                    SaveBitmap(bitmap, Variant(screenshotPath, "-gauge-" + percent + (scale > 1 ? "-150" : "")));
                AssertRingPixels(bitmap, scale, percent, trackColor, fillColor);
            }
        }

        settings.LowThreshold = 25;
        view.ApplySettings(settings);
        UsageSnapshot lowBattery = CreateSnapshot(2);
        lowBattery.Devices[0].BatteryPercent = 1;
        view.UpdateSnapshot(lowBattery);
        Arrange(view, 480, 900);
        Color warning = ((SolidColorBrush)((System.Windows.Shapes.Path)Gauge(view, 0).Children[1]).Stroke).Color;
        System.Drawing.Color expectedWarning = IconRenderer.ColorForPercent(1, settings);
        Check(warning.R == expectedWarning.R && warning.G == expectedWarning.G && warning.B == expectedWarning.B,
            "low-battery ring retains its warning color");
        lowBattery.Devices[0].BatteryPercent = null;
        lowBattery.Devices[1].BatteryPercent = 0;
        view.UpdateSnapshot(lowBattery);
        Arrange(view, 480, 900);
        Check(Gauge(view, 0).Children.Count == 2 && Gauge(view, 1).Children.Count == 2,
            "unknown and empty batteries do not display a colored arc");
        Check(((Wpf.Ui.Controls.SymbolIcon)Gauge(view, 0).Children[1]).Symbol == Wpf.Ui.Controls.SymbolRegular.BatteryWarning20,
            "unknown battery displays its warning symbol");
        settings.LowThreshold = originalLowThreshold;
        view.ApplySettings(settings);
    }

    private static void AssertRingPixels(BitmapSource bitmap, double scale, int percent, Color track, Color fill)
    {
        int stride = bitmap.PixelWidth * 4;
        byte[] pixels = new byte[stride * bitmap.PixelHeight];
        bitmap.CopyPixels(pixels, stride, 0);
        for (int angle = 0; angle < 360; angle += 15)
        {
            double radians = (angle - 90) * Math.PI / 180;
            int x = (int)Math.Round((20 + 16 * Math.Cos(radians)) * scale);
            int y = (int)Math.Round((20 + 16 * Math.Sin(radians)) * scale);
            Check(NearColor(pixels, bitmap.PixelWidth, bitmap.PixelHeight, stride, x, y, track, scale) ||
                NearColor(pixels, bitmap.PixelWidth, bitmap.PixelHeight, stride, x, y, fill, scale),
                "rendered ring follows the common centerline at " + percent + "%, angle " + angle + ", scale " + scale);
        }
        double fillAngle = (percent * 1.8 - 90) * Math.PI / 180;
        int fillX = (int)Math.Round((20 + 16 * Math.Cos(fillAngle)) * scale);
        int fillY = (int)Math.Round((20 + 16 * Math.Sin(fillAngle)) * scale);
        Check(NearColor(pixels, bitmap.PixelWidth, bitmap.PixelHeight, stride, fillX, fillY, fill, scale),
            "colored arc renders on the track at " + percent + "% and scale " + scale);
        if (percent <= 95)
        {
            double trackAngle = ((percent * 3.6 + 360) / 2 - 90) * Math.PI / 180;
            int trackX = (int)Math.Round((20 + 16 * Math.Cos(trackAngle)) * scale);
            int trackY = (int)Math.Round((20 + 16 * Math.Sin(trackAngle)) * scale);
            Check(NearColor(pixels, bitmap.PixelWidth, bitmap.PixelHeight, stride, trackX, trackY, track, scale),
                "unfilled track renders on the common centerline at " + percent + "% and scale " + scale);
        }
    }

    private static bool NearColor(byte[] pixels, int width, int height, int stride, int x, int y, Color expected, double scale)
    {
        int range = Math.Max(1, (int)Math.Ceiling(scale));
        for (int py = Math.Max(0, y - range); py <= Math.Min(height - 1, y + range); py++)
            for (int px = Math.Max(0, x - range); px <= Math.Min(width - 1, x + range); px++)
            {
                int offset = py * stride + px * 4;
                int alpha = pixels[offset + 3];
                if (alpha < 220) continue;
                int blue = pixels[offset] * 255 / alpha;
                int green = pixels[offset + 1] * 255 / alpha;
                int red = pixels[offset + 2] * 255 / alpha;
                if (Math.Abs(red - expected.R) < 25 && Math.Abs(green - expected.G) < 25 && Math.Abs(blue - expected.B) < 25) return true;
            }
        return false;
    }

    private static string DeviceIconName(PopupView view, int index)
    {
        Grid grid = (Grid)Row(view, index).Child;
        return AutomationProperties.GetName((DependencyObject)grid.Children[0]);
    }

    private static bool HasText(DependencyObject root, string text) { return FindText(root, text) != null; }
    private static FrameworkElement ArrangeToolTip(ToolTip tooltip)
    {
        tooltip.Measure(new Size(400, double.PositiveInfinity));
        tooltip.Arrange(new Rect(tooltip.DesiredSize));
        tooltip.UpdateLayout();
        return (FrameworkElement)tooltip.Content;
    }

    private static TextBlock FindText(DependencyObject root, string text)
    {
        TextBlock value = root as TextBlock;
        if (value != null && value.Text == text) return value;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
        {
            TextBlock found = FindText(VisualTreeHelper.GetChild(root, i), text);
            if (found != null) return found;
        }
        return null;
    }

    private static object Field(object value, string name)
    {
        return value.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic).GetValue(value);
    }

    private static void Capture(PopupView view, string path, double scale)
    {
        RenderTargetBitmap bitmap = RenderVisual(view, view.ActualWidth, view.ActualHeight, scale);
        SaveBitmap(bitmap, path);
    }

    private static RenderTargetBitmap RenderVisual(Visual visual, double width, double height, double scale)
    {
        RenderTargetBitmap bitmap = new RenderTargetBitmap((int)Math.Ceiling(width * scale), (int)Math.Ceiling(height * scale),
            96 * scale, 96 * scale, PixelFormats.Pbgra32);
        bitmap.Render(visual);
        return bitmap;
    }

    private static void SaveBitmap(BitmapSource bitmap, string path)
    {
        PngBitmapEncoder encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        using (FileStream file = File.Create(path)) encoder.Save(file);
    }

    private static string Variant(string path, string suffix)
    {
        return Path.Combine(Path.GetDirectoryName(path), Path.GetFileNameWithoutExtension(path) + suffix + ".png");
    }

    private static UsageSnapshot CreateSnapshot(int count)
    {
        string[] names = { "HyperX Cloud III S Wireless", "PRO X SUPERLIGHT 2c", "Wireless Earbuds", "Game Controller" };
        string[] statuses = { "Connected", "Charging", "Connected", "Connected" };
        string[] estimates = { "Est. -3.2%/h · ~14h 30m remaining", "Est. +36%/h · ~1h 15m to full", "Est. -1.5%/h · ~47h 20m remaining", "Est. -0.8%/h · ~110h remaining" };
        int[] levels = { 47, 55, 72, 88 };
        UsageSnapshot result = new UsageSnapshot { LastUpdated = new DateTime(2026, 11, 3, 11, 57, 0) };
        for (int i = 0; i < count; i++)
        {
            int j = i % names.Length;
            result.Devices.Add(new LimitWindow { DeviceId = "device-" + i, DisplayName = names[j], DetailLeft = "Wireless", DetailRight = statuses[j], BatteryPercent = levels[j], BatteryEstimate = estimates[j] });
        }
        result.Weekly = result.Devices[0];
        result.FiveHour = count > 1 ? result.Devices[1] : null;
        return result;
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new Exception(message);
    }
}
