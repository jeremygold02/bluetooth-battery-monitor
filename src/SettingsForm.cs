using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Windows.Forms;

namespace CodexUsageTray
{
    internal sealed partial class SettingsForm : Form
    {
        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attribute, ref int value, int size);

        private readonly AppSettings settings;
        private readonly ToolTip toolTip;

        private CheckBox colorBarsCheckBox;
        private NumericUpDown criticalNumeric;
        private NumericUpDown lowNumeric;
        private NumericUpDown refreshNumeric;
        private NumericUpDown idleRefreshNumeric;
        private NumericUpDown previewPercentNumeric;
        private RadioButton weeklyRadio;
        private RadioButton fiveHourRadio;
        private RadioButton deviceRadio;
        private ComboBox deviceCombo;
        private ComboBox themeCombo;
        private CheckBox showResetTimesCheckBox;
        private CheckBox showLastUpdatedCheckBox;
        private CheckBox showResetAvailabilityCheckBox;
        private CheckBox startWithWindowsCheckBox;
        private CheckBox thresholdNotificationsCheckBox;
        private CheckBox showTrayBoxCheckBox;
        private PictureBox trayPreview;
        private Button trayBoxColorButton;
        private Button trayBoxAutoButton;
        private Button trayTextColorButton;
        private Button checkUpdatesButton;
        private Label trayBoxHexLabel;
        private Label trayTextHexLabel;
        private Label contrastStatusLabel;
        private Label updateStatusLabel;
        private bool useCustomTrayBoxColor;
        private Color trayBoxColor;
        private Color trayTextColor;
        private bool adjustingValues;
        private Panel[] settingsPages;
        private Button[] navigationButtons;
        private int selectedPage = 2;
        private Font pageHeadingFont;
        private Font cardHeadingFont;
        private Font mutedFont;
        private Font bodyFont;
        private Font navigationIconFont;
        private SettingsView settingsView;
        private AppSettings savedEditableSettings;
        private bool lastUnsavedChanges;
        private bool suppressUnsavedChangeEvents;
        private bool modalSettingsEditInProgress;

        public event EventHandler SettingsApplied;
        public event EventHandler UpdateCheckRequested;
        internal event EventHandler UnsavedChangesChanged;

        internal bool HasUnsavedChanges
        {
            get
            {
                if (savedEditableSettings == null) return false;
                if (modalSettingsEditInProgress) return true;
                if (deviceRadio.Checked && deviceCombo.SelectedItem == null) return true;
                return !EditableSettingsEqual(savedEditableSettings, CreateSettingsCandidate()) ||
                    (settingsView != null && settingsView.HasPendingNumericInput());
            }
        }

        public SettingsForm(AppSettings settings, UsageSnapshot snapshot)
        {
            if (settings == null)
            {
                throw new ArgumentNullException("settings");
            }

            this.settings = settings;
            toolTip = new ToolTip();
            toolTip.AutomaticDelay = 350;
            toolTip.AutoPopDelay = 12000;
            toolTip.ShowAlways = true;

            Text = "Bluetooth Battery Monitor Settings";
            ClientSize = new Size(960, 640);
            MinimumSize = new Size(640, 420);
            FormBorderStyle = FormBorderStyle.Sizable;
            MaximizeBox = false;
            MinimizeBox = false;
            ShowIcon = true;
            ShowInTaskbar = true;
            StartPosition = FormStartPosition.CenterScreen;
            bodyFont = new Font("Segoe UI", 12.0F, FontStyle.Regular, GraphicsUnit.Point);
            Font = bodyFont;
            pageHeadingFont = new Font("Segoe UI", 19.0F, FontStyle.Bold, GraphicsUnit.Point);
            cardHeadingFont = new Font("Segoe UI", 12.0F, FontStyle.Bold, GraphicsUnit.Point);
            mutedFont = new Font("Segoe UI", 11.5F, FontStyle.Regular, GraphicsUnit.Point);
            navigationIconFont = new Font("Segoe MDL2 Assets", 14.0F, FontStyle.Regular, GraphicsUnit.Point);
            AutoScaleDimensions = new SizeF(96.0F, 96.0F);
            AutoScaleMode = AutoScaleMode.Dpi;

            SetWindowIcon();
            BuildUi();
            UpdateDevices(snapshot);
            LoadSettings();
            WireEvents();
            ApplyCurrentTheme();
            UpdatePreviews();
            BuildWpfSettingsView();
            savedEditableSettings = CreateSettingsCandidate();
            WireUnsavedChangesEvents();
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing)
            {
                if (trayPreview != null && trayPreview.Image != null)
                {
                    Image image = trayPreview.Image;
                    trayPreview.Image = null;
                    image.Dispose();
                }
                toolTip.Dispose();
                pageHeadingFont.Dispose();
                cardHeadingFont.Dispose();
                mutedFont.Dispose();
                navigationIconFont.Dispose();
                bodyFont.Dispose();
            }

            base.Dispose(disposing);
        }

        protected override void OnHandleCreated(EventArgs e)
        {
            base.OnHandleCreated(e);
            ApplyTitleBarTheme();
        }

        private void ApplyTitleBarTheme()
        {
            if (!IsHandleCreated)
            {
                return;
            }
            bool dark = !SystemInformation.HighContrast &&
                AppSettings.IsDarkTheme(themeCombo == null ? settings.Theme : GetSelectedTheme());
            int darkMode = dark ? 1 : 0;
            int caption = dark ? ColorTranslator.ToWin32(Color.FromArgb(22, 22, 22)) : -1;
            int text = dark ? ColorTranslator.ToWin32(Color.White) : -1;
            try
            {
                DwmSetWindowAttribute(Handle, 20, ref darkMode, sizeof(int));
                DwmSetWindowAttribute(Handle, 35, ref caption, sizeof(int));
                DwmSetWindowAttribute(Handle, 36, ref text, sizeof(int));
            }
            catch (DllNotFoundException)
            {
            }
            catch (EntryPointNotFoundException)
            {
            }
        }

        private void SetWindowIcon()
        {
            try
            {
                Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath);
            }
            catch
            {
            }
        }

        private void BuildUi()
        {
            SuspendLayout();
            // Construct the existing controls first, then arrange them on the new pages.
            // Keeping their existing event handlers also keeps color and update behavior intact.
            Panel legacy = new Panel();
            legacy.Visible = false;
            legacy.Controls.Add(BuildUsageGroup());
            legacy.Controls.Add(BuildRefreshGroup());
            legacy.Controls.Add(BuildAppearanceGroup());
            legacy.Controls.Add(BuildPopupGroup());
            legacy.Controls.Add(BuildUpdatesGroup());
            Controls.Add(legacy);

            TableLayoutPanel root = new TableLayoutPanel();
            root.Dock = DockStyle.Fill;
            root.ColumnCount = 2;
            root.RowCount = 2;
            root.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 205));
            root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            root.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
            root.RowStyles.Add(new RowStyle(SizeType.Absolute, 68));
            root.Margin = Padding.Empty;
            root.Tag = "body";

            FlowLayoutPanel sidebar = new FlowLayoutPanel();
            sidebar.FlowDirection = FlowDirection.TopDown;
            sidebar.WrapContents = false;
            sidebar.Dock = DockStyle.Fill;
            sidebar.Padding = new Padding(12, 25, 12, 0);
            sidebar.Margin = Padding.Empty;
            sidebar.Tag = "sidebar";
            string[] names = { "General", "Battery & alerts", "Tray icon", "Appearance" };
            string[] glyphs = { "\uE713", "\uE7F4", "\uE850", "\uE790" };
            navigationButtons = new Button[names.Length];
            for (int index = 0; index < names.Length; index++)
            {
                int page = index;
                Button button = new Button();
                button.Name = "settingsNav" + index;
                button.AccessibleName = names[index];
                button.Text = names[index].Replace("&", "&&");
                button.TextAlign = ContentAlignment.MiddleLeft;
                button.Padding = new Padding(44, 0, 2, 0);
                button.Size = new Size(180, 44);
                button.Margin = new Padding(0, 0, 0, 6);
                button.FlatStyle = FlatStyle.Flat;
                button.FlatAppearance.BorderSize = 0;
                button.Tag = "nav";
                button.Click += delegate { ShowSettingsPage(page); };
                Label icon = new Label();
                icon.Font = navigationIconFont;
                icon.Text = glyphs[index];
                icon.TextAlign = ContentAlignment.MiddleCenter;
                icon.SetBounds(9, 7, 28, 28);
                icon.Click += delegate { ShowSettingsPage(page); };
                button.Controls.Add(icon);
                sidebar.Controls.Add(button);
                navigationButtons[index] = button;
            }
            root.Controls.Add(sidebar, 0, 0);
            root.SetRowSpan(sidebar, 2);

            Panel pageHost = new Panel();
            pageHost.Dock = DockStyle.Fill;
            pageHost.Margin = Padding.Empty;
            pageHost.Tag = "body";
            settingsPages = new Panel[4];
            settingsPages[0] = BuildGeneralPage();
            settingsPages[1] = BuildAlertsPage();
            settingsPages[2] = BuildTrayPage();
            settingsPages[3] = BuildAppearancePage();
            for (int index = 0; index < settingsPages.Length; index++)
            {
                pageHost.Controls.Add(settingsPages[index]);
            }
            root.Controls.Add(pageHost, 1, 0);

            TableLayoutPanel footer = new TableLayoutPanel();
            footer.Dock = DockStyle.Fill;
            footer.Padding = new Padding(20, 12, 20, 10);
            footer.ColumnCount = 2;
            footer.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            footer.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            footer.Tag = "body";
            Button defaultsButton = CreateButton("&Defaults", "Restore default settings", 100);
            defaultsButton.Click += delegate { RestoreDefaults(); };
            toolTip.SetToolTip(defaultsButton, "Restore the controls to their default values. Changes are saved only after Save.");
            footer.Controls.Add(defaultsButton, 0, 0);
            FlowLayoutPanel actionButtons = CreateFlowLayout();
            actionButtons.FlowDirection = FlowDirection.RightToLeft;
            actionButtons.Anchor = AnchorStyles.Right;
            Button cancelButton = CreateButton("&Cancel", "Cancel settings changes", 100);
            cancelButton.DialogResult = DialogResult.Cancel;
            cancelButton.Click += delegate { Close(); };
            actionButtons.Controls.Add(cancelButton);
            Button saveButton = CreateButton("&Save", "Save settings", 100);
            saveButton.Tag = "save";
            saveButton.DialogResult = DialogResult.OK;
            saveButton.Click += delegate
            {
                if (!TryApplyToSettings())
                {
                    DialogResult = DialogResult.None;
                    return;
                }
                Close();
            };
            actionButtons.Controls.Add(saveButton);
            footer.Controls.Add(actionButtons, 1, 0);
            root.Controls.Add(footer, 1, 1);
            AcceptButton = saveButton;
            CancelButton = cancelButton;
            Controls.Add(root);
            ShowSettingsPage(selectedPage);
            ResumeLayout(false);
        }

        private GroupBox BuildUsageGroup()
        {
            GroupBox group = CreateGroup("Battery && alerts");
            TableLayoutPanel layout = CreateGroupLayout(1);

            thresholdNotificationsCheckBox = CreateCheckBox(
                "&Notify at thresholds",
                "Notify at thresholds");
            toolTip.SetToolTip(
                thresholdNotificationsCheckBox,
                "Show an alert when a device battery first reaches the low or critical threshold.");
            layout.Controls.Add(thresholdNotificationsCheckBox, 0, 0);

            FlowLayoutPanel thresholdRow = CreateFlowLayout();
            thresholdRow.Controls.Add(CreateRowLabel("&Critical", "Critical threshold"));
            criticalNumeric = CreateNumeric(1, 99, 66, "Critical threshold percent");
            thresholdRow.Controls.Add(criticalNumeric);
            thresholdRow.Controls.Add(CreateSuffixLabel("%"));

            Label lowLabel = CreateRowLabel("&Low", "Low threshold");
            lowLabel.Margin = new Padding(16, 3, 3, 1);
            thresholdRow.Controls.Add(lowLabel);
            lowNumeric = CreateNumeric(1, 99, 66, "Low threshold percent");
            thresholdRow.Controls.Add(lowNumeric);
            thresholdRow.Controls.Add(CreateSuffixLabel("%"));
            toolTip.SetToolTip(criticalNumeric, "Critical must be less than or equal to Low. The paired value adjusts immediately.");
            toolTip.SetToolTip(lowNumeric, "Low must be greater than or equal to Critical. The paired value adjusts immediately.");
            layout.Controls.Add(thresholdRow, 0, 1);

            FlowLayoutPanel metricRow = CreateFlowLayout();
            metricRow.Controls.Add(CreateRowLabel("Tray &icon:", "Tray icon metric"));
            weeklyRadio = CreateRadio("&Lowest", "Lowest device battery");
            fiveHourRadio = CreateRadio("&Average", "Average device battery");
            deviceRadio = CreateRadio("&Specific device", "Specific device battery");
            metricRow.Controls.Add(weeklyRadio);
            metricRow.Controls.Add(fiveHourRadio);
            metricRow.Controls.Add(deviceRadio);
            toolTip.SetToolTip(weeklyRadio, "Show the lowest known device battery in the tray icon.");
            toolTip.SetToolTip(fiveHourRadio, "Show the average known device battery in the tray icon.");
            toolTip.SetToolTip(deviceRadio, "Show only the selected device in the tray icon.");
            layout.Controls.Add(metricRow, 0, 2);

            FlowLayoutPanel deviceRow = CreateFlowLayout();
            deviceRow.Controls.Add(CreateRowLabel("&Device:", "Device for tray icon"));
            deviceCombo = new ComboBox();
            deviceCombo.AccessibleName = "Device for tray icon";
            deviceCombo.DropDownStyle = ComboBoxStyle.DropDownList;
            deviceCombo.Anchor = AnchorStyles.Left;
            deviceCombo.Width = 245;
            deviceCombo.DropDownWidth = 320;
            deviceRow.Controls.Add(deviceCombo);
            toolTip.SetToolTip(deviceCombo, "Choose the device whose battery appears in the tray icon.");
            layout.Controls.Add(deviceRow, 0, 3);

            group.Controls.Add(layout);
            return group;
        }

        private GroupBox BuildRefreshGroup()
        {
            GroupBox group = CreateGroup("Polling");
            FlowLayoutPanel row = CreateFlowLayout();
            row.Dock = DockStyle.Top;

            row.Controls.Add(CreateRowLabel("Check &every", "Battery check interval"));
            refreshNumeric = CreateNumeric(30, 3600, 70, "Battery check interval in seconds");
            row.Controls.Add(refreshNumeric);
            row.Controls.Add(CreateSuffixLabel("sec"));
            toolTip.SetToolTip(refreshNumeric, "Check connected device batteries every 30 to 3600 seconds.");

            idleRefreshNumeric = CreateNumeric(0, 7200, 70, "Idle refresh interval in seconds");
            idleRefreshNumeric.Visible = false;

            group.Controls.Add(row);
            return group;
        }

        private GroupBox BuildAppearanceGroup()
        {
            GroupBox group = CreateGroup("Appearance");
            TableLayoutPanel layout = CreateGroupLayout(1);
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100.0F));

            TableLayoutPanel themeRow = CreateGroupLayout(2);
            themeRow.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            themeRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100.0F));

            Label themeLabel = CreateRowLabel("&Theme", "Application theme");
            themeLabel.MinimumSize = new Size(70, 0);
            themeRow.Controls.Add(themeLabel, 0, 0);
            themeCombo = new ComboBox();
            themeCombo.AccessibleName = "Application theme";
            themeCombo.Anchor = AnchorStyles.Left | AnchorStyles.Right;
            themeCombo.DropDownStyle = ComboBoxStyle.DropDownList;
            themeCombo.FlatStyle = FlatStyle.Flat;
            themeCombo.Margin = new Padding(3, 1, 3, 1);
            themeCombo.Items.AddRange(new object[]
            {
                AppSettings.ThemeSystem,
                AppSettings.ThemeDark,
                AppSettings.ThemeLight
            });
            themeRow.Controls.Add(themeCombo, 1, 0);
            toolTip.SetToolTip(themeCombo, "Use the Windows app theme, dark mode, or light mode.");
            layout.Controls.Add(themeRow, 0, 0);

            TableLayoutPanel optionsRow = CreateGroupLayout(2);
            optionsRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50.0F));
            optionsRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50.0F));

            colorBarsCheckBox = CreateCheckBox(
                "Color by &battery level",
                "Color by battery level");
            optionsRow.Controls.Add(colorBarsCheckBox, 0, 0);

            showTrayBoxCheckBox = CreateCheckBox(
                "Show tray &background",
                "Show tray background");
            optionsRow.Controls.Add(showTrayBoxCheckBox, 1, 0);
            layout.Controls.Add(optionsRow, 0, 1);

            TableLayoutPanel colorLayout = CreateGroupLayout(4);
            colorLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            colorLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            colorLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            colorLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100.0F));

            colorLayout.Controls.Add(CreateRowLabel("&Background", "Tray background color"), 0, 0);
            trayBoxColorButton = CreateColorButton("Choose tray background color");
            trayBoxColorButton.Click += delegate { PickTrayBoxColor(); };
            colorLayout.Controls.Add(trayBoxColorButton, 1, 0);

            trayBoxHexLabel = CreateValueLabel("Tray background color value");
            trayBoxAutoButton = CreateButton("&Auto", "Use automatic tray background color", 54);
            trayBoxAutoButton.Margin = new Padding(3, 1, 6, 1);
            trayBoxAutoButton.Anchor = AnchorStyles.Left;
            trayBoxAutoButton.Click += delegate
            {
                useCustomTrayBoxColor = false;
                UpdatePreviews();
            };
            colorLayout.Controls.Add(trayBoxAutoButton, 2, 0);

            colorLayout.Controls.Add(trayBoxHexLabel, 3, 0);

            colorLayout.Controls.Add(CreateRowLabel("&Text", "Tray text color"), 0, 1);
            trayTextColorButton = CreateColorButton("Choose tray text color");
            trayTextColorButton.Click += delegate { PickTrayTextColor(); };
            colorLayout.Controls.Add(trayTextColorButton, 1, 1);

            trayTextHexLabel = CreateValueLabel("Tray text color value");
            colorLayout.Controls.Add(trayTextHexLabel, 3, 1);
            layout.Controls.Add(colorLayout, 0, 2);

            FlowLayoutPanel previewRow = CreateFlowLayout();
            Label previewLabel = CreateRowLabel("Live tray &preview", "Live tray preview");
            previewLabel.Margin = new Padding(3, 9, 5, 1);
            previewRow.Controls.Add(previewLabel);

            trayPreview = new PictureBox();
            trayPreview.AccessibleName = "Live tray preview";
            trayPreview.AccessibleRole = AccessibleRole.Graphic;
            trayPreview.BackColor = SystemColors.ControlDark;
            trayPreview.BorderStyle = BorderStyle.FixedSingle;
            trayPreview.Margin = new Padding(3, 1, 8, 1);
            trayPreview.Size = new Size(34, 34);
            trayPreview.SizeMode = PictureBoxSizeMode.CenterImage;
            previewRow.Controls.Add(trayPreview);

            Label remainingLabel = CreateRowLabel("&Battery", "Preview battery level");
            remainingLabel.Margin = new Padding(3, 9, 3, 1);
            previewRow.Controls.Add(remainingLabel);
            previewPercentNumeric = CreateNumeric(0, 100, 62, "Preview battery percent");
            previewPercentNumeric.Value = 50;
            previewPercentNumeric.Margin = new Padding(3, 5, 3, 1);
            previewRow.Controls.Add(previewPercentNumeric);
            Label previewSuffix = CreateSuffixLabel("%");
            previewSuffix.Margin = new Padding(0, 9, 3, 1);
            previewRow.Controls.Add(previewSuffix);
            toolTip.SetToolTip(previewPercentNumeric, "Change this sample percentage to preview normal, low, and critical tray states.");
            layout.Controls.Add(previewRow, 0, 3);

            contrastStatusLabel = CreateValueLabel("Tray text contrast guidance");
            contrastStatusLabel.MaximumSize = new Size(340, 0);
            contrastStatusLabel.Margin = new Padding(3, 1, 3, 1);
            layout.Controls.Add(contrastStatusLabel, 0, 4);

            group.Controls.Add(layout);
            return group;
        }

        private GroupBox BuildPopupGroup()
        {
            GroupBox group = CreateGroup("Popup && startup");
            TableLayoutPanel layout = CreateGroupLayout(2);
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50.0F));
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50.0F));

            showResetAvailabilityCheckBox = CreateCheckBox(
                "Show &more devices",
                "Show additional devices");
            layout.Controls.Add(showResetAvailabilityCheckBox, 0, 0);

            showResetTimesCheckBox = CreateCheckBox(
                "Show device &details",
                "Show device connection details");
            layout.Controls.Add(showResetTimesCheckBox, 1, 0);

            showLastUpdatedCheckBox = CreateCheckBox(
                "Show &updated time",
                "Show updated time");
            layout.Controls.Add(showLastUpdatedCheckBox, 0, 1);

            startWithWindowsCheckBox = CreateCheckBox(
                "&Start with Windows",
                "Start with Windows");
            layout.Controls.Add(startWithWindowsCheckBox, 1, 1);

            group.Controls.Add(layout);
            return group;
        }

        private GroupBox BuildUpdatesGroup()
        {
            GroupBox group = CreateGroup("About");
            TableLayoutPanel layout = CreateGroupLayout(2);
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100.0F));

            Label versionLabel = CreateValueLabel("Installed version");
            versionLabel.Text = "Version " + AppVersion.Current;
            versionLabel.Margin = new Padding(3, 4, 8, 1);
            layout.Controls.Add(versionLabel, 0, 0);

            updateStatusLabel = CreateValueLabel("Update status");
            updateStatusLabel.AutoEllipsis = true;
            updateStatusLabel.AutoSize = false;
            updateStatusLabel.Dock = DockStyle.Fill;
            updateStatusLabel.Margin = new Padding(3, 3, 8, 1);
            updateStatusLabel.MinimumSize = new Size(0, 24);
            updateStatusLabel.TextAlign = ContentAlignment.MiddleLeft;
            updateStatusLabel.Text = "Updates have not been checked yet.";
            layout.Controls.Add(updateStatusLabel, 1, 0);

            checkUpdatesButton = CreateButton("Check for updates", "Check for updates", 112);
            checkUpdatesButton.Visible = false;

            group.Controls.Add(layout);
            return group;
        }

        private Panel CreateSettingsPage(string title, string description, out TableLayoutPanel content)
        {
            Panel page = new Panel();
            page.Dock = DockStyle.Fill;
            page.AutoScroll = true;
            page.AutoScrollMinSize = new Size(700, 0);
            page.Margin = Padding.Empty;
            page.Tag = "body";
            content = new TableLayoutPanel();
            content.ColumnCount = 1;
            content.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            content.AutoSize = true;
            content.AutoSizeMode = AutoSizeMode.GrowAndShrink;
            content.MinimumSize = new Size(700, 0);
            content.Dock = DockStyle.Top;
            content.Padding = new Padding(24, 22, 24, 18);
            content.Margin = Padding.Empty;
            content.Tag = "body";
            Label heading = new Label();
            heading.Text = title;
            heading.UseMnemonic = false;
            heading.AutoSize = true;
            heading.Font = pageHeadingFont;
            heading.Margin = new Padding(0, 0, 0, 1);
            content.Controls.Add(heading);
            Label subtitle = new Label();
            subtitle.Text = description;
            subtitle.AutoSize = true;
            subtitle.Font = mutedFont;
            subtitle.Tag = "muted";
            subtitle.Margin = new Padding(0, 0, 0, 18);
            content.Controls.Add(subtitle);
            page.Controls.Add(content);
            return page;
        }

        private SettingsCard NewCard(int height)
        {
            SettingsCard card = new SettingsCard();
            card.Height = height;
            card.Dock = DockStyle.Top;
            card.Margin = new Padding(0, 0, 0, 12);
            card.Tag = "card";
            return card;
        }

        private Label CardLabel(string text, int x, int y, bool heading)
        {
            Label label = new Label();
            label.Text = text;
            label.AutoSize = true;
            label.Font = heading ? cardHeadingFont : mutedFont;
            label.Location = new Point(x, y);
            label.Tag = heading ? "cardtext" : "cardmuted";
            label.UseMnemonic = false;
            return label;
        }

        private static void PlaceControl(Control parent, Control control, int x, int y)
        {
            parent.Controls.Add(control);
            control.Dock = DockStyle.None;
            control.Anchor = AnchorStyles.Top | AnchorStyles.Left;
            control.Location = new Point(x, y);
            control.Visible = true;
        }

        private Panel BuildGeneralPage()
        {
            TableLayoutPanel content;
            Panel page = CreateSettingsPage("General", "Choose how often devices are checked and how the app starts.", out content);
            SettingsCard polling = NewCard(110);
            polling.Controls.Add(CardLabel("Battery checks", 20, 14, true));
            polling.Controls.Add(CardLabel("Check connected devices at this interval.", 20, 40, false));
            PlaceControl(polling, refreshNumeric, 24, 72);
            refreshNumeric.Size = new Size(80, 24);
            polling.Controls.Add(CardLabel("seconds", 112, 76, false));
            content.Controls.Add(polling);
            SettingsCard startup = NewCard(92);
            startup.Controls.Add(CardLabel("Startup", 20, 14, true));
            PlaceControl(startup, startWithWindowsCheckBox, 24, 53);
            content.Controls.Add(startup);
            SettingsCard about = NewCard(92);
            about.Controls.Add(CardLabel("About", 20, 14, true));
            about.Controls.Add(CardLabel("Version " + AppVersion.Current, 24, 52, false));
            updateStatusLabel.AutoSize = true;
            updateStatusLabel.Tag = "cardmuted";
            PlaceControl(about, updateStatusLabel, 155, 52);
            content.Controls.Add(about);
            return page;
        }

        private Panel BuildAlertsPage()
        {
            TableLayoutPanel content;
            Panel page = CreateSettingsPage("Battery & alerts", "Notifications and battery warning thresholds.", out content);
            SettingsCard notifications = NewCard(98);
            notifications.Controls.Add(CardLabel("Battery notifications", 20, 14, true));
            notifications.Controls.Add(CardLabel("Show an alert when a device reaches a threshold.", 20, 39, false));
            PlaceControl(notifications, thresholdNotificationsCheckBox, 24, 68);
            content.Controls.Add(notifications);
            SettingsCard thresholds = NewCard(112);
            thresholds.Controls.Add(CardLabel("Battery thresholds", 20, 14, true));
            thresholds.Controls.Add(CardLabel("Critical level", 24, 62, false));
            PlaceControl(thresholds, criticalNumeric, 120, 58);
            criticalNumeric.Size = new Size(70, 24);
            thresholds.Controls.Add(CardLabel("%", 194, 62, false));
            thresholds.Controls.Add(CardLabel("Low level", 255, 62, false));
            PlaceControl(thresholds, lowNumeric, 325, 58);
            lowNumeric.Size = new Size(70, 24);
            thresholds.Controls.Add(CardLabel("%", 399, 62, false));
            content.Controls.Add(thresholds);
            return page;
        }

        private Panel BuildAppearancePage()
        {
            TableLayoutPanel content;
            Panel page = CreateSettingsPage("Appearance", "Choose which details appear in the battery popup.", out content);
            SettingsCard appearance = NewCard(130);
            appearance.Controls.Add(CardLabel("Battery colors", 20, 14, true));
            PlaceControl(appearance, colorBarsCheckBox, 24, 53);
            appearance.Controls.Add(CardLabel("Use the warning thresholds to color battery indicators.", 46, 77, false));
            content.Controls.Add(appearance);
            SettingsCard popup = NewCard(170);
            popup.Controls.Add(CardLabel("Popup details", 20, 14, true));
            PlaceControl(popup, showResetAvailabilityCheckBox, 24, 50);
            PlaceControl(popup, showResetTimesCheckBox, 24, 79);
            PlaceControl(popup, showLastUpdatedCheckBox, 24, 108);
            content.Controls.Add(popup);
            return page;
        }

        private Panel BuildTrayPage()
        {
            TableLayoutPanel content;
            Panel page = CreateSettingsPage("Tray icon", "Customize how the battery level is shown in the system tray.", out content);
            SettingsCard metric = NewCard(220);
            weeklyRadio.Text = "&Lowest battery";
            PlaceControl(metric, weeklyRadio, 26, 18);
            metric.Controls.Add(CardLabel("Show the lowest battery level across your devices.", 49, 40, false));
            fiveHourRadio.Text = "&Average battery";
            PlaceControl(metric, fiveHourRadio, 26, 72);
            metric.Controls.Add(CardLabel("Show the average battery level across your devices.", 49, 94, false));
            deviceRadio.Text = "&Specific device";
            PlaceControl(metric, deviceRadio, 26, 126);
            metric.Controls.Add(CardLabel("Show the battery level for a chosen device.", 49, 148, false));
            PlaceControl(metric, deviceCombo, 49, 177);
            deviceCombo.Size = new Size(390, 28);
            deviceCombo.DropDownWidth = 420;
            content.Controls.Add(metric);

            SettingsCard preview = NewCard(198);
            preview.Controls.Add(CardLabel("LIVE PREVIEW", 22, 15, false));
            PlaceControl(preview, trayPreview, 42, 51);
            trayPreview.Size = new Size(54, 54);
            preview.Controls.Add(CardLabel("Battery", 24, 135, false));
            PlaceControl(preview, previewPercentNumeric, 81, 130);
            previewPercentNumeric.Size = new Size(62, 24);
            preview.Controls.Add(CardLabel("%", 145, 135, false));
            PlaceControl(preview, showTrayBoxCheckBox, 190, 14);
            preview.Controls.Add(CardLabel("Background color", 190, 51, false));
            PlaceControl(preview, trayBoxAutoButton, 320, 44);
            trayBoxAutoButton.AutoSize = false;
            trayBoxAutoButton.Size = new Size(64, 28);
            PlaceControl(preview, trayBoxColorButton, 390, 44);
            trayBoxColorButton.Size = new Size(38, 28);
            AddPresetSwatch(preview, 436, 44, Color.FromArgb(25, 29, 32), true, "dark graphite");
            AddPresetSwatch(preview, 468, 44, Color.FromArgb(0, 88, 83), true, "dark teal");
            AddPresetSwatch(preview, 500, 44, Color.FromArgb(0, 196, 185), true, "teal");
            Button customBackground = CreateButton("Custom...", "Choose custom tray background color", 82);
            customBackground.AutoSize = false;
            customBackground.SetBounds(534, 44, 100, 28);
            customBackground.Click += delegate { PickTrayBoxColor(); };
            preview.Controls.Add(customBackground);
            trayBoxHexLabel.AutoSize = true;
            trayBoxHexLabel.Tag = "cardmuted";
            PlaceControl(preview, trayBoxHexLabel, 190, 78);

            preview.Controls.Add(CardLabel("Text color", 190, 108, false));
            PlaceControl(preview, trayTextColorButton, 320, 101);
            trayTextColorButton.Size = new Size(38, 28);
            AddPresetSwatch(preview, 366, 101, Color.White, false, "white");
            AddPresetSwatch(preview, 398, 101, Color.LightGray, false, "light gray");
            AddPresetSwatch(preview, 430, 101, Color.Black, false, "black");
            Button customText = CreateButton("Custom...", "Choose custom tray text color", 82);
            customText.AutoSize = false;
            customText.SetBounds(464, 101, 100, 28);
            customText.Click += delegate { PickTrayTextColor(); };
            preview.Controls.Add(customText);
            trayTextHexLabel.AutoSize = true;
            trayTextHexLabel.Tag = "cardmuted";
            PlaceControl(preview, trayTextHexLabel, 190, 136);
            PlaceControl(preview, contrastStatusLabel, 190, 164);
            contrastStatusLabel.Size = new Size(450, 25);
            contrastStatusLabel.MaximumSize = new Size(440, 0);
            contrastStatusLabel.Tag = "cardmuted";
            content.Controls.Add(preview);

            SettingsCard theme = NewCard(80);
            theme.Controls.Add(CardLabel("Theme", 20, 13, true));
            theme.Controls.Add(CardLabel("Choose the app color theme.", 20, 40, false));
            PlaceControl(theme, themeCombo, 360, 29);
            themeCombo.Size = new Size(230, 28);
            content.Controls.Add(theme);
            return page;
        }

        private void AddPresetSwatch(Control parent, int x, int y, Color color, bool background, string name)
        {
            Button swatch = new Button();
            swatch.AccessibleName = "Set tray " + (background ? "background" : "text") + " color to " + name;
            swatch.Tag = "swatch";
            swatch.FlatStyle = FlatStyle.Flat;
            swatch.UseVisualStyleBackColor = false;
            swatch.BackColor = color;
            swatch.ForeColor = ContrastColor(color);
            swatch.SetBounds(x, y, 26, 28);
            swatch.Click += delegate
            {
                if (background)
                {
                    trayBoxColor = color;
                    useCustomTrayBoxColor = true;
                }
                else
                {
                    trayTextColor = color;
                }
                UpdatePreviews();
            };
            toolTip.SetToolTip(swatch, swatch.AccessibleName);
            parent.Controls.Add(swatch);
        }

        private void ShowSettingsPage(int page)
        {
            if (settingsPages == null || page < 0 || page >= settingsPages.Length)
            {
                return;
            }
            selectedPage = page;
            for (int index = 0; index < settingsPages.Length; index++)
            {
                settingsPages[index].Visible = index == page;
            }
            settingsPages[page].BringToFront();
            UpdateNavigationTheme();
        }

        private void UpdateNavigationTheme()
        {
            UpdateNavigationTheme(SystemInformation.HighContrast);
        }

        private void UpdateNavigationTheme(bool highContrast)
        {
            if (navigationButtons == null)
            {
                return;
            }
            bool dark = AppSettings.IsDarkTheme(themeCombo == null ? settings.Theme : GetSelectedTheme());
            for (int index = 0; index < navigationButtons.Length; index++)
            {
                Button button = navigationButtons[index];
                bool active = index == selectedPage;
                button.FlatAppearance.BorderSize = 0;
                button.BackColor = highContrast
                    ? (active ? SystemColors.Highlight : SystemColors.Control)
                    : active ? (dark ? Color.FromArgb(19, 67, 68) : Color.FromArgb(210, 239, 237))
                    : (dark ? Color.FromArgb(20, 23, 26) : SystemColors.Control);
                button.ForeColor = highContrast && active ? SystemColors.HighlightText
                    : dark && !highContrast ? Color.White : SystemColors.ControlText;
                foreach (Control child in button.Controls)
                {
                    child.BackColor = button.BackColor;
                    child.ForeColor = button.ForeColor;
                }
            }
        }

        private sealed class DeviceChoice
        {
            public string Id;
            public string Name;
            public string Label;

            public override string ToString()
            {
                return Label;
            }
        }

        public void UpdateDevices(UsageSnapshot snapshot)
        {
            DeviceChoice current = deviceCombo.SelectedItem as DeviceChoice;
            string selectedId = current != null ? current.Id : settings.IconDeviceId;
            string selectedName = current != null ? current.Name : settings.IconDeviceName;
            List<LimitWindow> devices = new List<LimitWindow>();
            if (snapshot != null && snapshot.Devices != null)
            {
                foreach (LimitWindow device in snapshot.Devices)
                {
                    if (device != null && !string.IsNullOrWhiteSpace(device.DeviceId))
                    {
                        devices.Add(device);
                    }
                }
            }
            devices.Sort(delegate(LimitWindow left, LimitWindow right)
            {
                int nameOrder = string.Compare(left.DisplayName, right.DisplayName,
                    StringComparison.CurrentCultureIgnoreCase);
                return nameOrder != 0 ? nameOrder : string.Compare(left.DeviceId, right.DeviceId,
                    StringComparison.OrdinalIgnoreCase);
            });

            List<DeviceChoice> choices = new List<DeviceChoice>();
            HashSet<string> ids = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (LimitWindow device in devices)
            {
                if (!ids.Add(device.DeviceId))
                {
                    continue;
                }
                string name = string.IsNullOrWhiteSpace(device.DisplayName) ? "Wireless device" : device.DisplayName;
                string label = name;
                if (!string.IsNullOrWhiteSpace(device.DetailLeft))
                {
                    label += " (" + device.DetailLeft + ")";
                }
                int sameLabelCount = 0;
                foreach (DeviceChoice previous in choices)
                {
                    if (string.Equals(previous.Name, name, StringComparison.CurrentCultureIgnoreCase))
                    {
                        sameLabelCount++;
                    }
                }
                if (sameLabelCount > 0)
                {
                    label += " #" + (sameLabelCount + 1).ToString();
                }
                if (device.IsCached || !device.BatteryPercent.HasValue)
                {
                    label += " (unavailable)";
                }
                choices.Add(new DeviceChoice { Id = device.DeviceId, Name = name, Label = label });
            }

            if (!string.IsNullOrEmpty(selectedId) && !ids.Contains(selectedId))
            {
                string name = string.IsNullOrWhiteSpace(selectedName) ? "Selected device" : selectedName;
                choices.Add(new DeviceChoice { Id = selectedId, Name = name, Label = name + " (unavailable)" });
            }

            bool wasSuppressingUnsavedChanges = suppressUnsavedChangeEvents;
            suppressUnsavedChangeEvents = true;
            deviceCombo.BeginUpdate();
            try
            {
                deviceCombo.Items.Clear();
                int selectedIndex = -1;
                foreach (DeviceChoice choice in choices)
                {
                    int index = deviceCombo.Items.Add(choice);
                    if (string.Equals(choice.Id, selectedId, StringComparison.OrdinalIgnoreCase))
                    {
                        selectedIndex = index;
                    }
                }
                deviceCombo.SelectedIndex = selectedIndex >= 0 ? selectedIndex :
                    (deviceCombo.Items.Count > 0 ? 0 : -1);
            }
            finally
            {
                deviceCombo.EndUpdate();
                suppressUnsavedChangeEvents = wasSuppressingUnsavedChanges;
            }
            if (settingsView != null) settingsView.RefreshDevices();
            RefreshUnsavedChanges();
        }

        private void LoadSettings()
        {
            adjustingValues = true;
            SetNumericValue(criticalNumeric, settings.CriticalThreshold);
            SetNumericValue(lowNumeric, settings.LowThreshold);
            SetNumericValue(refreshNumeric, settings.RefreshSeconds);
            SetNumericValue(idleRefreshNumeric, settings.IdleRefreshSeconds);
            EnsureValidValues();

            weeklyRadio.Checked = !string.Equals(settings.IconMetric, AppSettings.IconMetricFiveHour,
                StringComparison.OrdinalIgnoreCase) &&
                !string.Equals(settings.IconMetric, AppSettings.IconMetricDevice, StringComparison.OrdinalIgnoreCase);
            fiveHourRadio.Checked = string.Equals(
                settings.IconMetric,
                AppSettings.IconMetricFiveHour,
                StringComparison.OrdinalIgnoreCase);
            deviceRadio.Checked = string.Equals(settings.IconMetric, AppSettings.IconMetricDevice,
                StringComparison.OrdinalIgnoreCase);
            deviceCombo.Enabled = deviceRadio.Checked;
            colorBarsCheckBox.Checked = settings.ColorBars;
            showResetTimesCheckBox.Checked = settings.ShowPopupResetTimes;
            showLastUpdatedCheckBox.Checked = settings.ShowPopupLastUpdated;
            showResetAvailabilityCheckBox.Checked = settings.ShowResetAvailability;
            startWithWindowsCheckBox.Checked = StartupManager.IsEnabled();
            thresholdNotificationsCheckBox.Checked = settings.ThresholdNotifications;
            showTrayBoxCheckBox.Checked = settings.ShowTrayBox;
            useCustomTrayBoxColor = settings.UseCustomTrayBoxColor;
            trayBoxColor = settings.GetTrayBoxColor();
            trayTextColor = settings.GetTrayTextColor();
            SelectTheme(settings.Theme);
            adjustingValues = false;
        }

        private void WireEvents()
        {
            deviceRadio.CheckedChanged += delegate { deviceCombo.Enabled = deviceRadio.Checked; };
            criticalNumeric.ValueChanged += delegate
            {
                if (adjustingValues)
                {
                    return;
                }
                adjustingValues = true;
                if (criticalNumeric.Value > lowNumeric.Value)
                {
                    lowNumeric.Value = criticalNumeric.Value;
                }
                adjustingValues = false;
                UpdatePreviews();
            };

            lowNumeric.ValueChanged += delegate
            {
                if (adjustingValues)
                {
                    return;
                }
                adjustingValues = true;
                if (lowNumeric.Value < criticalNumeric.Value)
                {
                    criticalNumeric.Value = lowNumeric.Value;
                }
                adjustingValues = false;
                UpdatePreviews();
            };

            refreshNumeric.ValueChanged += delegate
            {
                if (adjustingValues)
                {
                    return;
                }
                adjustingValues = true;
                if (idleRefreshNumeric.Value > 0 && idleRefreshNumeric.Value < refreshNumeric.Value)
                {
                    idleRefreshNumeric.Value = refreshNumeric.Value;
                }
                adjustingValues = false;
            };

            idleRefreshNumeric.ValueChanged += delegate
            {
                if (adjustingValues)
                {
                    return;
                }
                adjustingValues = true;
                if (idleRefreshNumeric.Value > 0 && idleRefreshNumeric.Value < refreshNumeric.Value)
                {
                    idleRefreshNumeric.Value = refreshNumeric.Value;
                }
                adjustingValues = false;
            };

            themeCombo.SelectedIndexChanged += delegate
            {
                if (!adjustingValues)
                {
                    ApplyCurrentTheme();
                    UpdatePreviews();
                }
            };
            showTrayBoxCheckBox.CheckedChanged += delegate { UpdatePreviews(); };
            previewPercentNumeric.ValueChanged += delegate { UpdatePreviews(); };
        }

        private void WireUnsavedChangesEvents()
        {
            EventHandler changed = delegate { RefreshUnsavedChanges(); };
            foreach (CheckBox box in new CheckBox[] { colorBarsCheckBox, showResetTimesCheckBox,
                showLastUpdatedCheckBox, showResetAvailabilityCheckBox, startWithWindowsCheckBox,
                thresholdNotificationsCheckBox, showTrayBoxCheckBox }) box.CheckedChanged += changed;
            foreach (RadioButton radio in new RadioButton[] { weeklyRadio, fiveHourRadio, deviceRadio })
                radio.CheckedChanged += changed;
            foreach (NumericUpDown number in new NumericUpDown[] { criticalNumeric, lowNumeric, refreshNumeric, idleRefreshNumeric })
                number.ValueChanged += changed;
            deviceCombo.SelectedIndexChanged += changed;
            themeCombo.SelectedIndexChanged += changed;
        }

        private void RefreshUnsavedChanges()
        {
            if (savedEditableSettings == null || adjustingValues || suppressUnsavedChangeEvents) return;
            bool dirty = HasUnsavedChanges;
            if (dirty == lastUnsavedChanges) return;
            lastUnsavedChanges = dirty;
            if (UnsavedChangesChanged != null) UnsavedChangesChanged(this, EventArgs.Empty);
        }

        private static bool EditableSettingsEqual(AppSettings first, AppSettings second)
        {
            return first.CriticalThreshold == second.CriticalThreshold &&
                first.LowThreshold == second.LowThreshold &&
                string.Equals(first.IconMetric, second.IconMetric, StringComparison.OrdinalIgnoreCase) &&
                (!string.Equals(first.IconMetric, AppSettings.IconMetricDevice, StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(first.IconDeviceId, second.IconDeviceId, StringComparison.OrdinalIgnoreCase)) &&
                first.ColorBars == second.ColorBars && first.ShowPopupResetTimes == second.ShowPopupResetTimes &&
                first.ShowPopupLastUpdated == second.ShowPopupLastUpdated &&
                first.ShowResetAvailability == second.ShowResetAvailability &&
                first.StartWithWindows == second.StartWithWindows &&
                first.ThresholdNotifications == second.ThresholdNotifications &&
                first.ShowTrayBox == second.ShowTrayBox &&
                first.UseCustomTrayBoxColor == second.UseCustomTrayBoxColor &&
                (!first.UseCustomTrayBoxColor ||
                    string.Equals(first.TrayBoxColor, second.TrayBoxColor, StringComparison.OrdinalIgnoreCase)) &&
                string.Equals(first.TrayTextColor, second.TrayTextColor, StringComparison.OrdinalIgnoreCase) &&
                first.RefreshSeconds == second.RefreshSeconds && first.IdleRefreshSeconds == second.IdleRefreshSeconds &&
                string.Equals(first.Theme, second.Theme, StringComparison.OrdinalIgnoreCase);
        }

        private bool TryApplyToSettings()
        {
            return TryApplyToSettingsUsing(delegate(AppSettings candidate) { candidate.Save(); });
        }

        internal bool TryApplyToSettingsUsing(Action<AppSettings> saveCandidate)
        {
            if (settings.IsSaveBlockedByNewerVersion)
            {
                MessageBox.Show(
                    this,
                    settings.SaveBlockedMessage,
                    "Bluetooth Battery Settings",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
                return false;
            }

            EnsureValidValues();
            DeviceChoice selectedDevice = deviceCombo.SelectedItem as DeviceChoice;
            if (deviceRadio.Checked && selectedDevice == null)
            {
                MessageBox.Show(this, "Choose a device for the tray icon.", "Bluetooth Battery Settings",
                    MessageBoxButtons.OK, MessageBoxIcon.Information);
                return false;
            }
            AppSettings candidate = CreateSettingsCandidate();

            try
            {
                saveCandidate(candidate);
            }
            catch (Exception ex)
            {
                MessageBox.Show(
                    this,
                    "Could not save settings:" + Environment.NewLine + Environment.NewLine + ex.Message,
                    "Bluetooth Battery Settings",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
                return false;
            }

            settings.CopyValuesFrom(candidate);

            if (SettingsApplied != null)
            {
                SettingsApplied(this, EventArgs.Empty);
            }

            savedEditableSettings = CreateSettingsCandidate();
            RefreshUnsavedChanges();

            return true;
        }

        private AppSettings CreateSettingsCandidate()
        {
            DeviceChoice selectedDevice = deviceCombo.SelectedItem as DeviceChoice;
            AppSettings candidate = settings.Clone();
            candidate.CriticalThreshold = (int)criticalNumeric.Value;
            candidate.LowThreshold = (int)lowNumeric.Value;
            candidate.IconMetric = deviceRadio.Checked ? AppSettings.IconMetricDevice :
                (fiveHourRadio.Checked ? AppSettings.IconMetricFiveHour : AppSettings.IconMetricWeekly);
            if (deviceRadio.Checked && selectedDevice != null)
            {
                candidate.IconDeviceId = selectedDevice.Id;
                candidate.IconDeviceName = selectedDevice.Name;
            }
            candidate.ColorBars = colorBarsCheckBox.Checked;
            candidate.ShowPopupResetTimes = showResetTimesCheckBox.Checked;
            candidate.ShowPopupLastUpdated = showLastUpdatedCheckBox.Checked;
            candidate.ShowResetAvailability = showResetAvailabilityCheckBox.Checked;
            candidate.StartWithWindows = startWithWindowsCheckBox.Checked;
            candidate.ThresholdNotifications = thresholdNotificationsCheckBox.Checked;
            candidate.ShowTrayBox = showTrayBoxCheckBox.Checked;
            candidate.UseCustomTrayBoxColor = useCustomTrayBoxColor;
            candidate.TrayBoxColor = AppSettings.FormatColor(trayBoxColor);
            candidate.TrayTextColor = AppSettings.FormatColor(trayTextColor);
            candidate.RefreshSeconds = (int)refreshNumeric.Value;
            candidate.IdleRefreshSeconds = (int)idleRefreshNumeric.Value;
            candidate.Theme = GetSelectedTheme();

            return candidate;
        }

        private void RestoreDefaults()
        {
            AppSettings defaults = new AppSettings();
            adjustingValues = true;
            criticalNumeric.Value = defaults.CriticalThreshold;
            lowNumeric.Value = defaults.LowThreshold;
            refreshNumeric.Value = defaults.RefreshSeconds;
            idleRefreshNumeric.Value = defaults.IdleRefreshSeconds;
            weeklyRadio.Checked = true;
            fiveHourRadio.Checked = false;
            deviceRadio.Checked = false;
            deviceCombo.Enabled = false;
            colorBarsCheckBox.Checked = defaults.ColorBars;
            showResetTimesCheckBox.Checked = defaults.ShowPopupResetTimes;
            showLastUpdatedCheckBox.Checked = defaults.ShowPopupLastUpdated;
            showResetAvailabilityCheckBox.Checked = defaults.ShowResetAvailability;
            startWithWindowsCheckBox.Checked = defaults.StartWithWindows;
            thresholdNotificationsCheckBox.Checked = defaults.ThresholdNotifications;
            showTrayBoxCheckBox.Checked = defaults.ShowTrayBox;
            useCustomTrayBoxColor = defaults.UseCustomTrayBoxColor;
            trayBoxColor = defaults.GetTrayBoxColor();
            trayTextColor = defaults.GetTrayTextColor();
            previewPercentNumeric.Value = 50;
            SelectTheme(defaults.Theme);
            adjustingValues = false;
            ApplyCurrentTheme();
            UpdatePreviews();
            if (settingsView != null) settingsView.RefreshFromOwner();
            RefreshUnsavedChanges();
        }

        public void SetUpdateButtonState(bool busy, string busyText)
        {
            if (checkUpdatesButton == null)
            {
                return;
            }

            checkUpdatesButton.Enabled = !busy;
            checkUpdatesButton.Text = busy && !string.IsNullOrWhiteSpace(busyText) ? busyText : "Check for updates";
            if (settingsView != null) settingsView.RefreshUpdateButtonState();
        }

        public void SetUpdateStatus(string status)
        {
            if (updateStatusLabel == null)
            {
                return;
            }

            updateStatusLabel.Text = string.IsNullOrWhiteSpace(status) ? "Updates have not been checked yet." : status;
            updateStatusLabel.AccessibleDescription = updateStatusLabel.Text;
            toolTip.SetToolTip(updateStatusLabel, updateStatusLabel.Text);
            if (settingsView != null) settingsView.RefreshStatus();
        }

        private void PickTrayBoxColor()
        {
            Color selected;
            if (TryPickColor(trayBoxColor, out selected))
            {
                trayBoxColor = selected;
                useCustomTrayBoxColor = true;
                UpdatePreviews();
            }
        }

        private void PickTrayTextColor()
        {
            Color selected;
            if (TryPickColor(trayTextColor, out selected))
            {
                trayTextColor = selected;
                UpdatePreviews();
            }
        }

        private bool TryPickColor(Color currentColor, out Color selectedColor)
        {
            SetModalSettingsEditInProgress(true);
            try
            {
                using (ColorDialog dialog = new ColorDialog())
                {
                    dialog.FullOpen = true;
                    dialog.Color = currentColor;
                    if (dialog.ShowDialog(this) == DialogResult.OK)
                    {
                        selectedColor = dialog.Color;
                        return true;
                    }
                }

                selectedColor = currentColor;
                return false;
            }
            finally { SetModalSettingsEditInProgress(false); }
        }

        private void SetModalSettingsEditInProgress(bool value)
        {
            modalSettingsEditInProgress = value;
            RefreshUnsavedChanges();
        }

        private void EnsureValidValues()
        {
            if (criticalNumeric.Value > lowNumeric.Value)
            {
                lowNumeric.Value = criticalNumeric.Value;
            }
            if (idleRefreshNumeric.Value > 0 && idleRefreshNumeric.Value < refreshNumeric.Value)
            {
                idleRefreshNumeric.Value = refreshNumeric.Value;
            }
        }

        private void UpdatePreviews()
        {
            if (trayPreview == null || previewPercentNumeric == null)
            {
                return;
            }

            int previewPercent = (int)previewPercentNumeric.Value;
            AppSettings previewSettings = CreatePreviewSettings();
            ReplacePreview(
                trayPreview,
                IconRenderer.CreatePreviewBitmap(previewPercent, previewSettings));

            Color effectiveBackground = useCustomTrayBoxColor
                ? trayBoxColor
                : IconRenderer.ColorForPercent(previewPercent, previewSettings);
            string effectiveBackgroundHex = AppSettings.FormatColor(effectiveBackground);
            string textHex = AppSettings.FormatColor(trayTextColor);

            trayBoxColorButton.FlatStyle = FlatStyle.Flat;
            trayBoxColorButton.UseVisualStyleBackColor = false;
            trayBoxColorButton.BackColor = effectiveBackground;
            trayBoxColorButton.ForeColor = ContrastColor(effectiveBackground);
            trayBoxColorButton.Text = useCustomTrayBoxColor ? "" : "A";
            trayBoxColorButton.AccessibleDescription = useCustomTrayBoxColor
                ? "Current tray background color " + effectiveBackgroundHex
                : "Automatic tray background color. Current preview " + effectiveBackgroundHex;

            trayTextColorButton.FlatStyle = FlatStyle.Flat;
            trayTextColorButton.UseVisualStyleBackColor = false;
            trayTextColorButton.BackColor = trayTextColor;
            trayTextColorButton.ForeColor = ContrastColor(trayTextColor);
            trayTextColorButton.Text = "";
            trayTextColorButton.AccessibleDescription = "Current tray text color " + textHex;

            trayBoxAutoButton.Enabled = useCustomTrayBoxColor;
            trayBoxHexLabel.Text = useCustomTrayBoxColor
                ? effectiveBackgroundHex
                : "Auto (" + effectiveBackgroundHex + ")";
            trayTextHexLabel.Text = textHex;

            toolTip.SetToolTip(
                trayBoxColorButton,
                useCustomTrayBoxColor
                    ? "Choose tray background color. Current: " + effectiveBackgroundHex
                    : "Choose a fixed tray background color. Current preview is automatic: " + effectiveBackgroundHex);
            toolTip.SetToolTip(
                trayBoxAutoButton,
                "Use automatic critical, low, and normal battery colors for the tray background.");
            toolTip.SetToolTip(
                trayTextColorButton,
                "Choose tray text color. Current: " + textHex);

            bool dark = AppSettings.IsDarkTheme(GetSelectedTheme());
            trayPreview.BackColor = SystemInformation.HighContrast
                ? SystemColors.ControlDark
                : dark ? Color.FromArgb(38, 38, 38) : Color.FromArgb(232, 232, 232);
            trayPreview.AccessibleDescription = previewPercent.ToString()
                + " percent remaining; background "
                + (showTrayBoxCheckBox.Checked ? effectiveBackgroundHex : "hidden")
                + "; text "
                + textHex;
            toolTip.SetToolTip(trayPreview, trayPreview.AccessibleDescription);

            UpdateContrastGuidance(effectiveBackground);
            if (settingsView != null) settingsView.RefreshPreview();
            RefreshUnsavedChanges();
        }

        private AppSettings CreatePreviewSettings()
        {
            AppSettings previewSettings = new AppSettings();
            previewSettings.CriticalThreshold = (int)criticalNumeric.Value;
            previewSettings.LowThreshold = (int)lowNumeric.Value;
            previewSettings.ShowTrayBox = showTrayBoxCheckBox.Checked;
            previewSettings.UseCustomTrayBoxColor = useCustomTrayBoxColor;
            previewSettings.TrayBoxColor = AppSettings.FormatColor(trayBoxColor);
            previewSettings.TrayTextColor = AppSettings.FormatColor(trayTextColor);
            return previewSettings;
        }

        private void UpdateContrastGuidance(Color effectiveBackground)
        {
            if (!showTrayBoxCheckBox.Checked)
            {
                contrastStatusLabel.Text = "Background hidden: use text that contrasts with the taskbar.";
                contrastStatusLabel.AccessibleDescription = contrastStatusLabel.Text;
                contrastStatusLabel.ForeColor = SystemInformation.HighContrast
                    ? SystemColors.ControlText
                    : ForeColor;
                toolTip.SetToolTip(contrastStatusLabel, contrastStatusLabel.Text);
                return;
            }

            double ratio = ContrastRatio(trayTextColor, effectiveBackground);
            double blackRatio = ContrastRatio(Color.Black, effectiveBackground);
            double whiteRatio = ContrastRatio(Color.White, effectiveBackground);
            Color recommended = blackRatio >= whiteRatio ? Color.Black : Color.White;
            string recommendedHex = AppSettings.FormatColor(recommended);
            bool lowContrast = ratio < 4.5;

            contrastStatusLabel.Text = (lowContrast ? "Low contrast " : "Contrast ")
                + ratio.ToString("0.00")
                + ":1. Recommended text: "
                + recommendedHex
                + ".";
            contrastStatusLabel.AccessibleDescription = contrastStatusLabel.Text;
            contrastStatusLabel.ForeColor = lowContrast
                ? GetWarningColor()
                : (SystemInformation.HighContrast ? SystemColors.ControlText : ForeColor);
            toolTip.SetToolTip(
                contrastStatusLabel,
                "A contrast ratio of 4.5:1 or higher is recommended for readable tray text.");
        }

        private Color GetWarningColor()
        {
            if (SystemInformation.HighContrast)
            {
                return SystemColors.ControlText;
            }

            return AppSettings.IsDarkTheme(GetSelectedTheme())
                ? Color.FromArgb(255, 205, 86)
                : Color.DarkRed;
        }

        private static double ContrastRatio(Color first, Color second)
        {
            double firstLuminance = RelativeLuminance(first);
            double secondLuminance = RelativeLuminance(second);
            double lighter = Math.Max(firstLuminance, secondLuminance);
            double darker = Math.Min(firstLuminance, secondLuminance);
            return (lighter + 0.05) / (darker + 0.05);
        }

        private static double RelativeLuminance(Color color)
        {
            return (0.2126 * LinearColorChannel(color.R))
                + (0.7152 * LinearColorChannel(color.G))
                + (0.0722 * LinearColorChannel(color.B));
        }

        private static double LinearColorChannel(byte channel)
        {
            double value = channel / 255.0;
            return value <= 0.03928
                ? value / 12.92
                : Math.Pow((value + 0.055) / 1.055, 2.4);
        }

        private static Color ContrastColor(Color color)
        {
            int brightness = ((color.R * 299) + (color.G * 587) + (color.B * 114)) / 1000;
            return brightness > 140 ? Color.Black : Color.White;
        }

        private static void ReplacePreview(PictureBox box, Bitmap bitmap)
        {
            Image old = box.Image;
            box.Image = bitmap;
            if (old != null)
            {
                old.Dispose();
            }
        }

        private string GetSelectedTheme()
        {
            object selected = themeCombo.SelectedItem;
            return selected != null ? selected.ToString() : AppSettings.ThemeSystem;
        }

        private void SelectTheme(string theme)
        {
            string normalized = string.IsNullOrEmpty(theme) ? AppSettings.ThemeSystem : theme;
            int index = themeCombo.Items.IndexOf(normalized);
            themeCombo.SelectedIndex = index >= 0 ? index : 0;
        }

        private void ApplyCurrentTheme()
        {
            ApplyTheme(this, AppSettings.IsDarkTheme(GetSelectedTheme()));
            UpdateNavigationTheme();
            ApplyTitleBarTheme();
            if (settingsView != null) settingsView.RefreshTheme();
        }

        private static void ApplyTheme(Control control, bool dark)
        {
            ApplyTheme(control, dark, SystemInformation.HighContrast);
        }

        private static void ApplyTheme(Control control, bool dark, bool highContrast)
        {
            Color backColor = highContrast
                ? SystemColors.Control
                : dark ? Color.FromArgb(54, 54, 54) : SystemColors.Control;
            Color foreColor = highContrast
                ? SystemColors.ControlText
                : dark ? Color.White : SystemColors.ControlText;
            Color inputBackColor = highContrast
                ? SystemColors.Window
                : dark ? Color.FromArgb(36, 36, 36) : SystemColors.Window;
            Color inputForeColor = highContrast
                ? SystemColors.WindowText
                : dark ? Color.White : SystemColors.WindowText;
            Color borderColor = highContrast
                ? SystemColors.WindowText
                : dark ? Color.FromArgb(100, 100, 100) : Color.FromArgb(170, 170, 170);

            string tag = control.Tag as string;
            bool inCard = tag == "card" || tag == "cardtext" || tag == "cardmuted" ||
                (control.Parent != null && string.Equals(control.Parent.Tag as string, "card", StringComparison.Ordinal));
            if (!highContrast)
            {
                backColor = inCard
                    ? (dark ? Color.FromArgb(31, 35, 38) : Color.FromArgb(251, 251, 251))
                    : tag == "sidebar"
                        ? (dark ? Color.FromArgb(20, 23, 26) : Color.FromArgb(237, 241, 241))
                        : dark ? Color.FromArgb(25, 29, 32) : Color.FromArgb(246, 248, 248);
                if (tag == "muted" || tag == "cardmuted")
                {
                    foreColor = dark ? Color.FromArgb(192, 199, 201) : Color.FromArgb(82, 90, 92);
                }
            }

            if (!(control is PictureBox) && tag != "swatch")
            {
                control.BackColor = backColor;
            }
            if (tag != "swatch")
            {
                control.ForeColor = foreColor;
            }

            DarkGroupBox group = control as DarkGroupBox;
            if (group != null)
            {
                group.BorderColor = borderColor;
                group.Invalidate();
            }

            SettingsCard card = control as SettingsCard;
            if (card != null)
            {
                card.BorderColor = highContrast ? SystemColors.WindowText :
                    dark ? Color.FromArgb(73, 79, 82) : Color.FromArgb(190, 198, 199);
                card.Invalidate();
            }

            Button button = control as Button;
            if (button != null && tag != "swatch")
            {
                if (highContrast)
                {
                    button.FlatStyle = FlatStyle.System;
                    button.UseVisualStyleBackColor = true;
                }
                else
                {
                    button.FlatStyle = FlatStyle.Flat;
                    button.UseVisualStyleBackColor = false;
                    button.BackColor = dark ? Color.FromArgb(74, 74, 74) : Color.FromArgb(245, 245, 245);
                    button.ForeColor = foreColor;
                    button.FlatAppearance.BorderColor = dark
                        ? Color.FromArgb(150, 150, 150)
                        : Color.FromArgb(170, 170, 170);
                    button.FlatAppearance.MouseOverBackColor = dark
                        ? Color.FromArgb(92, 92, 92)
                        : Color.FromArgb(229, 229, 229);
                    button.FlatAppearance.MouseDownBackColor = dark
                        ? Color.FromArgb(58, 58, 58)
                        : Color.FromArgb(214, 214, 214);
                    if (tag == "save")
                    {
                        button.BackColor = Color.FromArgb(0, 132, 126);
                        button.ForeColor = Color.White;
                        button.FlatAppearance.BorderColor = Color.FromArgb(0, 191, 181);
                    }
                }
            }

            foreach (Control child in control.Controls)
            {
                ApplyTheme(child, dark, highContrast);
                if (child is TextBox || child is ComboBox || child is NumericUpDown)
                {
                    child.BackColor = inputBackColor;
                    child.ForeColor = inputForeColor;
                }
            }
        }

        private static GroupBox CreateGroup(string text)
        {
            GroupBox group = new DarkGroupBox();
            group.AccessibleName = text;
            group.AutoSize = true;
            group.AutoSizeMode = AutoSizeMode.GrowAndShrink;
            group.Dock = DockStyle.Fill;
            group.Margin = new Padding(0, 0, 0, 4);
            group.Padding = new Padding(7, 3, 7, 5);
            group.Text = text;
            return group;
        }

        private static TableLayoutPanel CreateGroupLayout(int columns)
        {
            TableLayoutPanel layout = new TableLayoutPanel();
            layout.AutoSize = true;
            layout.AutoSizeMode = AutoSizeMode.GrowAndShrink;
            layout.ColumnCount = columns;
            layout.Dock = DockStyle.Top;
            layout.GrowStyle = TableLayoutPanelGrowStyle.AddRows;
            layout.Margin = Padding.Empty;
            layout.Padding = Padding.Empty;
            return layout;
        }

        private static FlowLayoutPanel CreateFlowLayout()
        {
            FlowLayoutPanel layout = new FlowLayoutPanel();
            layout.AutoSize = true;
            layout.AutoSizeMode = AutoSizeMode.GrowAndShrink;
            layout.FlowDirection = FlowDirection.LeftToRight;
            layout.Margin = Padding.Empty;
            layout.Padding = Padding.Empty;
            layout.WrapContents = false;
            return layout;
        }

        private static Label CreateRowLabel(string text, string accessibleName)
        {
            Label label = new Label();
            label.AccessibleName = accessibleName;
            label.Anchor = AnchorStyles.Left;
            label.AutoSize = true;
            label.Margin = new Padding(3, 3, 3, 1);
            label.Text = text;
            label.UseMnemonic = true;
            return label;
        }

        private static Label CreateSuffixLabel(string text)
        {
            Label label = new Label();
            label.Anchor = AnchorStyles.Left;
            label.AutoSize = true;
            label.Margin = new Padding(0, 3, 3, 1);
            label.Text = text;
            label.UseMnemonic = false;
            return label;
        }

        private static Label CreateValueLabel(string accessibleName)
        {
            Label label = new Label();
            label.AccessibleName = accessibleName;
            label.Anchor = AnchorStyles.Left;
            label.AutoSize = true;
            label.Margin = new Padding(3, 3, 3, 1);
            label.UseMnemonic = false;
            return label;
        }

        private static CheckBox CreateCheckBox(string text, string accessibleName)
        {
            CheckBox checkBox = new CheckBox();
            checkBox.AccessibleName = accessibleName;
            checkBox.Anchor = AnchorStyles.Left;
            checkBox.AutoSize = true;
            checkBox.Margin = new Padding(3, 1, 3, 1);
            checkBox.Text = text;
            checkBox.UseMnemonic = true;
            return checkBox;
        }

        private static RadioButton CreateRadio(string text, string accessibleName)
        {
            RadioButton radio = new SettingsRadioButton();
            radio.AccessibleName = accessibleName;
            radio.Anchor = AnchorStyles.Left;
            radio.AutoSize = false;
            radio.Size = new Size(390, 23);
            radio.Margin = new Padding(3, 1, 7, 1);
            radio.Text = text;
            radio.UseMnemonic = true;
            return radio;
        }

        private static NumericUpDown CreateNumeric(
            int minimum,
            int maximum,
            int width,
            string accessibleName)
        {
            NumericUpDown numeric = new NumericUpDown();
            numeric.AccessibleName = accessibleName;
            numeric.Anchor = AnchorStyles.Left;
            numeric.Margin = new Padding(3, 1, 3, 1);
            numeric.Minimum = minimum;
            numeric.Maximum = maximum;
            numeric.Size = new Size(width, 23);
            numeric.TextAlign = HorizontalAlignment.Right;
            return numeric;
        }

        private static Button CreateButton(string text, string accessibleName, int minimumWidth)
        {
            Button button = new Button();
            button.AccessibleName = accessibleName;
            button.AutoSize = true;
            button.AutoSizeMode = AutoSizeMode.GrowAndShrink;
            button.FlatStyle = FlatStyle.Flat;
            button.Margin = new Padding(4, 1, 0, 1);
            button.MinimumSize = new Size(minimumWidth, 28);
            button.Padding = new Padding(6, 0, 6, 0);
            button.Text = text;
            button.UseMnemonic = true;
            return button;
        }

        private static Button CreateColorButton(string accessibleName)
        {
            Button button = new Button();
            button.AccessibleName = accessibleName;
            button.Anchor = AnchorStyles.Left;
            button.FlatStyle = FlatStyle.Flat;
            button.Margin = new Padding(3, 1, 3, 1);
            button.Size = new Size(38, 28);
            button.UseVisualStyleBackColor = false;
            return button;
        }

        private static void SetNumericValue(NumericUpDown numeric, int value)
        {
            decimal clamped = Math.Max(numeric.Minimum, Math.Min(numeric.Maximum, value));
            numeric.Value = clamped;
        }
    }

    internal sealed class SettingsRadioButton : RadioButton
    {
        public SettingsRadioButton()
        {
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint |
                ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
        }

        protected override void OnCheckedChanged(EventArgs e)
        {
            base.OnCheckedChanged(e);
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            e.Graphics.Clear(BackColor);
            e.Graphics.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            int size = Math.Max(16, Math.Min(Height - 4, Font.Height));
            Rectangle circle = new Rectangle(1, (Height - size) / 2, size - 1, size - 1);
            Color line = !Enabled ? SystemColors.GrayText :
                Checked ? (SystemInformation.HighContrast ? SystemColors.Highlight : Color.FromArgb(0, 205, 193)) : ForeColor;
            using (Pen pen = new Pen(line, Math.Max(1.4F, size / 10.0F)))
            {
                e.Graphics.DrawEllipse(pen, circle);
            }
            if (Checked)
            {
                int inset = Math.Max(4, size / 4);
                Rectangle dot = Rectangle.Inflate(circle, -inset, -inset);
                using (Brush brush = new SolidBrush(line))
                {
                    e.Graphics.FillEllipse(brush, dot);
                }
            }
            Rectangle text = new Rectangle(size + 8, 0, Math.Max(0, Width - size - 8), Height);
            TextRenderer.DrawText(e.Graphics, Text, Font, text,
                Enabled ? ForeColor : SystemColors.GrayText,
                TextFormatFlags.SingleLine | TextFormatFlags.VerticalCenter |
                TextFormatFlags.NoPadding | TextFormatFlags.HidePrefix | TextFormatFlags.EndEllipsis);
            if (Focused && ShowFocusCues)
            {
                ControlPaint.DrawFocusRectangle(e.Graphics, text, ForeColor, BackColor);
            }
        }
    }

    internal sealed class SettingsCard : Panel
    {
        public Color BorderColor { get; set; }

        private System.Drawing.Drawing2D.GraphicsPath CardOutline()
        {
            System.Drawing.Drawing2D.GraphicsPath path = new System.Drawing.Drawing2D.GraphicsPath();
            int width = Math.Max(1, Width - 1);
            int height = Math.Max(1, Height - 1);
            int diameter = Math.Max(1, Math.Min(12, Math.Min(width, height)));
            path.AddArc(0, 0, diameter, diameter, 180, 90);
            path.AddArc(width - diameter, 0, diameter, diameter, 270, 90);
            path.AddArc(width - diameter, height - diameter, diameter, diameter, 0, 90);
            path.AddArc(0, height - diameter, diameter, diameter, 90, 90);
            path.CloseFigure();
            return path;
        }

        protected override void OnPaintBackground(PaintEventArgs e)
        {
            e.Graphics.Clear(Parent != null ? Parent.BackColor : BackColor);
            e.Graphics.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            using (System.Drawing.Drawing2D.GraphicsPath outline = CardOutline())
            using (Brush brush = new SolidBrush(BackColor))
            {
                e.Graphics.FillPath(brush, outline);
            }
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            e.Graphics.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            using (System.Drawing.Drawing2D.GraphicsPath outline = CardOutline())
            using (Pen pen = new Pen(BorderColor))
            {
                e.Graphics.DrawPath(pen, outline);
            }
        }
    }

    internal sealed class DarkGroupBox : GroupBox
    {
        public Color BorderColor { get; set; }

        public DarkGroupBox()
        {
            BorderColor = Color.FromArgb(100, 100, 100);
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            e.Graphics.Clear(BackColor);
            e.Graphics.TextRenderingHint = System.Drawing.Text.TextRenderingHint.ClearTypeGridFit;

            SizeF textSize = e.Graphics.MeasureString(Text, Font);
            int textX = 8;
            int borderY = Math.Max(8, (int)(textSize.Height / 2.0F));
            int textRight = textX + (int)Math.Ceiling(textSize.Width) + 8;

            using (Pen borderPen = new Pen(BorderColor))
            using (Brush textBrush = new SolidBrush(ForeColor))
            {
                e.Graphics.DrawLine(borderPen, 1, borderY, Math.Max(1, textX - 3), borderY);
                if (textRight < Width - 2)
                {
                    e.Graphics.DrawLine(borderPen, textRight, borderY, Width - 2, borderY);
                }
                e.Graphics.DrawLine(borderPen, 1, borderY, 1, Math.Max(borderY, Height - 2));
                e.Graphics.DrawLine(borderPen, Width - 2, borderY, Width - 2, Math.Max(borderY, Height - 2));
                e.Graphics.DrawLine(borderPen, 1, Height - 2, Width - 2, Height - 2);
                e.Graphics.DrawString(Text, Font, textBrush, textX, 0);
            }
        }
    }
}
