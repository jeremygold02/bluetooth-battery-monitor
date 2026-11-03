using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using System.Windows.Forms.Integration;

namespace CodexUsageTray
{
    // The tray remains a WinForms application; the popup content is laid out by WPF.
    internal sealed class UsagePopup : Form
    {
        private const double LogicalWidth = 480;
        private const int WmDpiChanged = 0x02E0;
        private readonly ElementHost host;
        private readonly PopupView popupView;
        private AppSettings settings;
        private Rectangle targetWorkingArea;
        private double dpiScale = 1;
        private bool updatingLayout;

        public event EventHandler RefreshRequested;
        public event EventHandler SettingsRequested;

        [DllImport("user32.dll")]
        private static extern IntPtr MonitorFromPoint(Point point, uint flags);

        [DllImport("shcore.dll")]
        private static extern int GetDpiForMonitor(IntPtr monitor, int dpiType, out uint dpiX, out uint dpiY);

        public UsagePopup(AppSettings settings)
        {
            this.settings = settings;
            Text = "Bluetooth Battery";
            FormBorderStyle = FormBorderStyle.None;
            ShowInTaskbar = false;
            TopMost = true;
            StartPosition = FormStartPosition.Manual;
            AutoScaleMode = AutoScaleMode.None;
            KeyPreview = true;

            popupView = new PopupView(settings);
            popupView.RefreshRequested += delegate { if (RefreshRequested != null) RefreshRequested(this, EventArgs.Empty); };
            popupView.SettingsRequested += delegate { if (SettingsRequested != null) SettingsRequested(this, EventArgs.Empty); };
            host = new ElementHost { Dock = DockStyle.Fill, Child = popupView, BackColor = PopupBackground() };
            Controls.Add(host);
            UpdateLayoutMetrics(false);
        }

        public void ApplySettings(AppSettings value)
        {
            settings = value;
            host.BackColor = PopupBackground();
            popupView.ApplySettings(value);
            UpdateLayoutMetrics(true);
        }

        public void UpdateSnapshot(UsageSnapshot value)
        {
            popupView.UpdateSnapshot(value);
            UpdateLayoutMetrics(true);
        }

        public void SetRefreshing(bool value)
        {
            popupView.SetRefreshing(value);
            UpdateLayoutMetrics(true);
        }

        public void ShowNear(Point cursor)
        {
            targetWorkingArea = Screen.FromPoint(cursor).WorkingArea;
            dpiScale = DpiScaleAt(cursor);
            UpdateLayoutMetrics(false);
            int margin = Scale(8);
            int x = cursor.X - Width + Scale(28);
            int y = cursor.Y - Height - Scale(18);
            if (y < targetWorkingArea.Top + margin) y = cursor.Y + Scale(18);
            Location = new Point(
                Math.Max(targetWorkingArea.Left + margin, Math.Min(x, targetWorkingArea.Right - Width - margin)),
                Math.Max(targetWorkingArea.Top + margin, Math.Min(y, targetWorkingArea.Bottom - Height - margin)));
            Show();
            Activate();
        }

        protected override void OnHandleCreated(EventArgs e)
        {
            base.OnHandleCreated(e);
            if (popupView != null)
            {
                using (Graphics graphics = CreateGraphics()) dpiScale = Math.Max(0.5, graphics.DpiX / 96.0);
                UpdateLayoutMetrics(false);
            }
        }

        protected override void OnDeactivate(EventArgs e) { base.OnDeactivate(e); Hide(); }

        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            if (e.CloseReason == CloseReason.UserClosing) { e.Cancel = true; Hide(); return; }
            base.OnFormClosing(e);
        }

        protected override void OnSystemColorsChanged(EventArgs e)
        {
            base.OnSystemColorsChanged(e);
            if (popupView != null) ApplySettings(settings);
        }

        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            if (keyData == Keys.Escape) { Hide(); return true; }
            if (keyData == Keys.Down) { popupView.ScrollBy(48); return true; }
            if (keyData == Keys.Up) { popupView.ScrollBy(-48); return true; }
            if (keyData == Keys.PageDown) { popupView.ScrollBy(popupView.DeviceScroll.ViewportHeight); return true; }
            if (keyData == Keys.PageUp) { popupView.ScrollBy(-popupView.DeviceScroll.ViewportHeight); return true; }
            if (keyData == Keys.Home) { popupView.ScrollToStart(); return true; }
            if (keyData == Keys.End) { popupView.ScrollToEnd(); return true; }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        protected override void WndProc(ref Message m)
        {
            bool changed = false;
            if (m.Msg == WmDpiChanged)
            {
                int dpi = (int)(m.WParam.ToInt64() & 0xFFFF);
                if (dpi > 0 && Math.Abs(dpiScale - dpi / 96.0) > 0.001) { dpiScale = dpi / 96.0; changed = true; }
            }
            base.WndProc(ref m);
            if (changed && popupView != null) UpdateLayoutMetrics(true);
        }

        protected override void OnResize(EventArgs e) { base.OnResize(e); UpdateRegion(); }

        private void UpdateLayoutMetrics(bool preserveBottom)
        {
            if (updatingLayout || popupView == null) return;
            updatingLayout = true;
            try
            {
                int bottom = Bottom;
                Rectangle area = targetWorkingArea.Width > 0 ? targetWorkingArea : Screen.FromControl(this).WorkingArea;
                int margin = Scale(8);
                int widthPixels = Math.Min(Scale((int)LogicalWidth), Math.Max(1, area.Width - margin * 2));
                double width = widthPixels / dpiScale;
                double maxHeight = Math.Max(1, (area.Height - margin * 2) / dpiScale);
                double height = popupView.MeasureHeight(width, maxHeight);
                ClientSize = new Size(widthPixels, Math.Min(Math.Max(1, area.Height - margin * 2), Math.Max(1, (int)Math.Ceiling(height * dpiScale))));
                UpdateRegion();
                if (preserveBottom && Visible)
                {
                    Top = bottom - Height;
                    Left = Math.Max(area.Left, Math.Min(Left, area.Right - Width));
                    Top = Math.Max(area.Top, Math.Min(Top, area.Bottom - Height));
                }
            }
            finally { updatingLayout = false; }
        }

        private Color PopupBackground()
        {
            if (SystemInformation.HighContrast) return SystemColors.Window;
            return settings == null || AppSettings.IsDarkTheme(settings.Theme) ? Color.FromArgb(22, 22, 22) : Color.FromArgb(244, 246, 248);
        }

        private int Scale(int value) { return (int)Math.Round(value * dpiScale); }

        private double DpiScaleAt(Point point)
        {
            try
            {
                uint x, y;
                IntPtr monitor = MonitorFromPoint(point, 2);
                if (monitor != IntPtr.Zero && GetDpiForMonitor(monitor, 0, out x, out y) == 0 && x > 0) return x / 96.0;
            }
            catch (DllNotFoundException) { }
            catch (EntryPointNotFoundException) { }
            return dpiScale;
        }

        private void UpdateRegion()
        {
            if (Width <= 1 || Height <= 1) return;
            int radius = Math.Max(1, Math.Min(Scale(9), Math.Min(Width, Height) / 2));
            int diameter = radius * 2;
            using (GraphicsPath path = new GraphicsPath())
            {
                path.AddArc(0, 0, diameter, diameter, 180, 90);
                path.AddArc(Width - diameter, 0, diameter, diameter, 270, 90);
                path.AddArc(Width - diameter, Height - diameter, diameter, diameter, 0, 90);
                path.AddArc(0, Height - diameter, diameter, diameter, 90, 90);
                path.CloseFigure();
                Region old = Region;
                Region = new Region(path);
                if (old != null) old.Dispose();
            }
        }
    }
}
