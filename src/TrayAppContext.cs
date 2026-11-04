using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.IO;
using System.Net;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using Microsoft.Win32;

namespace CodexUsageTray
{
    internal sealed class TrayAppContext : ApplicationContext
    {
        internal enum UpdateCheckMode { Startup, Periodic, Manual }
        internal enum UpdateCheckAction { None, InstallAutomatically, NotifyFailure, PromptInstall, PromptRelease }

        private const int ActivityPollSeconds = 30;
        private const int UpdateCheckIntervalMilliseconds = 12 * 60 * 60 * 1000;
        private const int ShowWindowRestore = 9;
        private const string RefreshingStatus = "Checking device batteries...";
        private const string RefreshFailedStatus = "Battery check failed; showing last known levels.";
        private const string RefreshFailedWithoutDataStatus = "Unable to check device batteries. Try again.";
        private const string PausedStatus = "Automatic battery checks are paused.";
        private const string PausedAfterFailureStatus = "Automatic checks paused; last battery check failed.";
        private const string WaitingStatus = "Waiting for supported wireless devices...";

        private readonly NotifyIcon notifyIcon;
        private readonly System.Windows.Forms.Timer refreshTimer;
        private readonly System.Windows.Forms.Timer activityTimer;
        private readonly System.Windows.Forms.Timer connectionDebounceTimer;
        private readonly System.Windows.Forms.Timer updateTimer;
        private BluetoothConnectionWatcher connectionWatcher;
        private readonly Control dispatcher;
        private readonly AppSettings settings;
        private readonly Stopwatch refreshClock;
        private readonly CancellationTokenSource shutdownCancellation;
        private readonly object updatePreparationLock = new object();
        private readonly HashSet<string> uninstallableUpdateNotices = new HashSet<string>(StringComparer.Ordinal);
        private readonly bool showUsageOnStart;
        private readonly bool showSettingsOnStart;

        private UsagePopup usagePopup;
        private Icon currentIcon;
        private UsageSnapshot lastSuccessfulSnapshot;
        private UsageSnapshot currentSnapshot;
        private int? lastWeeklyThresholdLevel;
        private int? lastFiveHourThresholdLevel;
        private long lastRefreshStartedMilliseconds = -1;
        private long savedBatteryRateRevision;
        private long savedBatteryCacheRevision;
        private DateTime lastAttemptedAt = DateTime.MinValue;
        private bool codexRunning;
        private bool refreshInProgress;
        private bool refreshFeedbackRequested;
        private bool connectionRefreshPending;
        private bool updateCheckInProgress;
        private bool updateInstallInProgress;
        private volatile bool shuttingDown;
        private string updateStatusText = "";
        private SettingsForm settingsForm;
        private System.Windows.Forms.Timer startupUiTimer;
        private System.Windows.Forms.Timer updateWaitTimer;
        private UpdateService.PreparedUpdate preparedUpdate;
        private bool preparedUpdateInteractive;

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool ShowWindow(IntPtr windowHandle, int command);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetForegroundWindow(IntPtr windowHandle);

        public TrayAppContext(string[] args)
        {
            showUsageOnStart = HasArg(args, "--show-usage");
            showSettingsOnStart = HasArg(args, "--show-settings");
            refreshClock = Stopwatch.StartNew();
            shutdownCancellation = new CancellationTokenSource();
            dispatcher = new Control();
            dispatcher.CreateControl();
            settings = AppSettings.Load();
            BatteryMonitorClient.InitializeBatteryRates(settings.LearnedBatteryRates);
            BatteryMonitorClient.InitializeBatteryCache(settings.CachedBatteryReadings);
            SystemEvents.PowerModeChanged += OnPowerModeChanged;
            usagePopup = CreateUsagePopup();

            notifyIcon = new NotifyIcon();
            SetNotifyTooltip("");
            notifyIcon.ContextMenuStrip = BuildContextMenu();
            notifyIcon.MouseClick += NotifyIcon_MouseClick;
            SetTrayIcon(IconRenderer.CreateUnknownIcon("..."));
            notifyIcon.Visible = true;

            refreshTimer = new System.Windows.Forms.Timer();
            refreshTimer.Tick += delegate
            {
                refreshTimer.Stop();
                RefreshUsageIfDue(false);
            };

            connectionDebounceTimer = new System.Windows.Forms.Timer();
            connectionDebounceTimer.Interval = 1500;
            connectionDebounceTimer.Tick += delegate
            {
                connectionDebounceTimer.Stop();
                if (refreshInProgress)
                {
                    connectionRefreshPending = true;
                }
                else
                {
                    RefreshUsage(false);
                }
            };
            try
            {
                connectionWatcher = new BluetoothConnectionWatcher(delegate
                {
                    TryPostToUi(delegate { QueueConnectionRefresh(); });
                });
                connectionWatcher.Start();
            }
            catch (Exception)
            {
                if (connectionWatcher != null)
                {
                    connectionWatcher.Dispose();
                    connectionWatcher = null;
                }
            }

            codexRunning = true;
            activityTimer = new System.Windows.Forms.Timer();
            activityTimer.Interval = ActivityPollSeconds * 1000;
            activityTimer.Tick += delegate { PollCodexActivity(); };
            activityTimer.Start();

            RefreshUsageIfDue(false);
            updateTimer = new System.Windows.Forms.Timer();
            updateTimer.Interval = UpdateCheckIntervalMilliseconds;
            updateTimer.Tick += delegate { CheckForUpdates(UpdateCheckMode.Periodic); };
            updateTimer.Start();
            if (HasArg(args, "--update-failed"))
            {
                SetUpdateStatusText("The previous update failed. The app was restored; check for updates to retry.");
                notifyIcon.ShowBalloonTip(5000, "Bluetooth Battery Monitor Update", updateStatusText, ToolTipIcon.Warning);
            }
            else if (HasArg(args, "--update-complete"))
            {
                SetUpdateStatusText("Updated to version " + AppVersion.Current + ".");
            }
            if (ShouldCheckForStartupUpdate(args))
            {
                CheckForUpdates(UpdateCheckMode.Startup);
            }
            if (showUsageOnStart || showSettingsOnStart)
            {
                startupUiTimer = new System.Windows.Forms.Timer();
                startupUiTimer.Interval = 2500;
                startupUiTimer.Tick += delegate
                {
                    startupUiTimer.Stop();
                    startupUiTimer.Dispose();
                    startupUiTimer = null;
                    if (showUsageOnStart)
                    {
                        ShowUsagePopup();
                    }
                    if (showSettingsOnStart)
                    {
                        ShowSettings();
                    }
                };
                startupUiTimer.Start();
            }
        }

        internal static bool ShouldCheckForStartupUpdate(string[] args)
        {
            return !HasArg(args, "--skip-startup-update");
        }

        private static bool HasArg(string[] args, string value)
        {
            if (args == null)
            {
                return false;
            }

            for (int i = 0; i < args.Length; i++)
            {
                if (string.Equals(args[i], value, StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }

            return false;
        }

        public void RequestExit()
        {
            if (shuttingDown)
            {
                return;
            }

            TryPostToUi(delegate
            {
                if (!shuttingDown)
                {
                    ExitThread();
                }
            });
        }

        private void OnPowerModeChanged(object sender, PowerModeChangedEventArgs e)
        {
            if (e.Mode == PowerModes.Resume)
            {
                BatteryMonitorClient.ResetBatteryRates();
                TryPostToUi(delegate { QueueConnectionRefresh(); });
            }
        }

        private void QueueConnectionRefresh()
        {
            if (shuttingDown)
            {
                return;
            }
            connectionDebounceTimer.Stop();
            connectionDebounceTimer.Start();
        }

        private ContextMenuStrip BuildContextMenu()
        {
            ContextMenuStrip menu = new ContextMenuStrip();

            ToolStripMenuItem refresh = new ToolStripMenuItem("Refresh now");
            refresh.Click += delegate { RefreshUsage(true); };
            menu.Items.Add(refresh);

            ToolStripMenuItem show = new ToolStripMenuItem("Show batteries");
            show.Click += delegate { ShowUsagePopup(); };
            menu.Items.Add(show);

            ToolStripMenuItem settingsItem = new ToolStripMenuItem("Settings");
            settingsItem.Click += delegate { ShowSettings(); };
            menu.Items.Add(settingsItem);

            ToolStripMenuItem checkUpdates = new ToolStripMenuItem("Check for updates");
            checkUpdates.Click += delegate { CheckForUpdates(UpdateCheckMode.Manual); };
            menu.Items.Add(checkUpdates);

            menu.Items.Add(new ToolStripSeparator());

            ToolStripMenuItem exit = new ToolStripMenuItem("Exit");
            exit.Click += delegate { ExitThread(); };
            menu.Items.Add(exit);

            return menu;
        }

        private void NotifyIcon_MouseClick(object sender, MouseEventArgs e)
        {
            if (e.Button == MouseButtons.Left)
            {
                ToggleUsagePopup();
            }
        }

        private void RefreshUsage(bool showBalloon)
        {
            if (shuttingDown)
            {
                return;
            }

            refreshFeedbackRequested = refreshFeedbackRequested || showBalloon;
            if (refreshInProgress)
            {
                SetUsagePopupRefreshing(true);
                return;
            }

            refreshInProgress = true;
            lastAttemptedAt = DateTime.Now;
            lastRefreshStartedMilliseconds = refreshClock.ElapsedMilliseconds;
            refreshTimer.Stop();
            ApplyRefreshingState();

            CancellationToken cancellationToken = shutdownCancellation.Token;
            Task.Factory.StartNew(
                delegate
                {
                    cancellationToken.ThrowIfCancellationRequested();
                    return BatteryMonitorClient.FetchUsage();
                },
                cancellationToken,
                TaskCreationOptions.None,
                TaskScheduler.Default).ContinueWith(delegate(Task<UsageSnapshot> task)
            {
                if (task.IsFaulted)
                {
                    AggregateException ignored = task.Exception;
                }

                TryPostToUi(delegate
                {
                    if (shuttingDown)
                    {
                        return;
                    }
                    CompleteRefresh(task);
                });
            }, CancellationToken.None, TaskContinuationOptions.None, TaskScheduler.Default);
        }

        private void RefreshUsageIfDue(bool showBalloon)
        {
            if (shuttingDown)
            {
                return;
            }

            codexRunning = true;
            int refreshSeconds = GetCurrentRefreshSeconds();
            if (refreshSeconds <= 0)
            {
                ApplyPausedState();
                return;
            }

            RestoreActiveState();
            if (refreshInProgress)
            {
                return;
            }

            if (lastRefreshStartedMilliseconds < 0)
            {
                RefreshUsage(showBalloon);
                return;
            }

            long intervalMilliseconds = (long)refreshSeconds * 1000L;
            long elapsedMilliseconds = refreshClock.ElapsedMilliseconds - lastRefreshStartedMilliseconds;
            long remainingMilliseconds = intervalMilliseconds - elapsedMilliseconds;
            if (remainingMilliseconds <= 0)
            {
                RefreshUsage(showBalloon);
                return;
            }

            ArmRefreshTimer(remainingMilliseconds);
        }

        private int GetCurrentRefreshSeconds()
        {
            if (codexRunning)
            {
                return Math.Max(30, settings.RefreshSeconds);
            }

            return Math.Max(0, settings.IdleRefreshSeconds);
        }

        private void ArmRefreshTimer(long delayMilliseconds)
        {
            if (shuttingDown || refreshInProgress)
            {
                return;
            }

            long boundedDelay = Math.Max(1L, Math.Min((long)int.MaxValue, delayMilliseconds));
            refreshTimer.Stop();
            refreshTimer.Interval = (int)boundedDelay;
            refreshTimer.Start();
        }

        private void PollCodexActivity()
        {
            if (shuttingDown)
            {
                return;
            }

            bool isRunning = true;
            if (isRunning == codexRunning)
            {
                return;
            }

            codexRunning = isRunning;
            RefreshScheduleChanged();
        }

        private void RefreshScheduleChanged()
        {
            refreshTimer.Stop();
            if (GetCurrentRefreshSeconds() <= 0)
            {
                ApplyPausedState();
                return;
            }

            RestoreActiveState();
            RefreshUsageIfDue(false);
        }

        private void CompleteRefresh(Task<UsageSnapshot> task)
        {
            refreshInProgress = false;
            bool showFeedback = refreshFeedbackRequested;
            refreshFeedbackRequested = false;
            SetUsagePopupRefreshing(false);

            UsageSnapshot failedSnapshot = !task.IsCanceled && !task.IsFaulted
                ? task.Result
                : null;
            if (failedSnapshot == null || !string.IsNullOrEmpty(failedSnapshot.ErrorMessage))
            {
                ApplyRefreshFailure(failedSnapshot, showFeedback);
            }
            else
            {
                PersistLearnedBatteryRates(task.Result);
                PersistBatteryCache(task.Result);
                ApplySuccessfulSnapshot(task.Result, showFeedback);
            }

            RefreshUsageIfDue(false);
            if (connectionRefreshPending)
            {
                connectionRefreshPending = false;
                QueueConnectionRefresh();
            }
        }

        private void PersistLearnedBatteryRates(UsageSnapshot snapshot)
        {
            long revision;
            List<LearnedBatteryRate> history;
            if (!BatteryMonitorClient.TryGetUpdatedBatteryRates(savedBatteryRateRevision, out revision, out history)) return;
            if (settings.TrySaveLearnedBatteryRates(history))
            {
                savedBatteryRateRevision = revision;
                return;
            }
            if (snapshot != null)
            {
                string warning = "Could not save learned battery rates; check the settings file.";
                snapshot.StatusMessage = string.IsNullOrEmpty(snapshot.StatusMessage)
                    ? warning : snapshot.StatusMessage + " " + warning;
            }
        }

        private void PersistBatteryCache(UsageSnapshot snapshot)
        {
            long revision;
            List<CachedBatteryReading> readings;
            if (!BatteryMonitorClient.TryGetUpdatedBatteryCache(savedBatteryCacheRevision, out revision, out readings)) return;
            if (settings.TrySaveBatteryCache(readings))
            {
                savedBatteryCacheRevision = revision;
                return;
            }
            if (snapshot != null)
            {
                string warning = "Could not save last known battery levels; check the settings file.";
                snapshot.StatusMessage = string.IsNullOrEmpty(snapshot.StatusMessage)
                    ? warning : snapshot.StatusMessage + " " + warning;
            }
        }

        private void ApplyRefreshingState()
        {
            UsageSnapshot refreshingSnapshot;
            if (lastSuccessfulSnapshot != null)
            {
                refreshingSnapshot = lastSuccessfulSnapshot.Clone();
            }
            else
            {
                refreshingSnapshot = UsageSnapshot.FromError(RefreshingStatus);
                refreshingSnapshot.LastUpdated = DateTime.MinValue;
            }

            refreshingSnapshot.LastAttempted = lastAttemptedAt;
            refreshingSnapshot.StatusMessage = RefreshingStatus;
            refreshingSnapshot.IsStale = false;
            refreshingSnapshot.IsRefreshing = true;
            refreshingSnapshot.IsPaused = false;
            currentSnapshot = refreshingSnapshot;

            if (lastSuccessfulSnapshot != null)
            {
                SetNotifyTooltip(BuildNativeTooltip(refreshingSnapshot));
            }
            else
            {
                SetNotifyTooltip("");
            }
            UpdateUsagePopup(refreshingSnapshot);
            SetUsagePopupRefreshing(true);
        }

        private void ApplySuccessfulSnapshot(UsageSnapshot snapshot, bool showBalloon)
        {
            if (snapshot.LastUpdated == DateTime.MinValue)
            {
                snapshot.LastUpdated = DateTime.Now;
            }
            snapshot.LastAttempted = lastAttemptedAt;
            snapshot.IsStale = false;
            snapshot.IsRefreshing = false;
            snapshot.IsPaused = false;

            lastSuccessfulSnapshot = snapshot.Clone();
            currentSnapshot = snapshot.Clone();
            RenderDataSnapshot(currentSnapshot, true, true, showBalloon);
        }

        private void ApplyRefreshFailure(UsageSnapshot failure, bool showBalloon)
        {
            string failureMessage = failure != null && !string.IsNullOrWhiteSpace(failure.ErrorMessage)
                ? failure.ErrorMessage
                : RefreshFailedWithoutDataStatus;
            if (lastSuccessfulSnapshot != null)
            {
                UsageSnapshot staleSnapshot = lastSuccessfulSnapshot.Clone();
                staleSnapshot.LastAttempted = lastAttemptedAt;
                staleSnapshot.ErrorMessage = failureMessage;
                staleSnapshot.StatusMessage = failureMessage;
                staleSnapshot.IsStale = true;
                staleSnapshot.IsRefreshing = false;
                staleSnapshot.IsPaused = false;
                currentSnapshot = staleSnapshot;

                SetNotifyTooltip(BuildNativeTooltip(staleSnapshot));
                UpdateUsagePopup(staleSnapshot);
            }
            else
            {
                UsageSnapshot failedSnapshot = failure != null
                    ? failure.Clone()
                    : UsageSnapshot.FromError(failureMessage);
                failedSnapshot.LastUpdated = DateTime.MinValue;
                failedSnapshot.LastAttempted = lastAttemptedAt;
                failedSnapshot.ErrorMessage = failureMessage;
                failedSnapshot.StatusMessage = failureMessage;
                failedSnapshot.IsStale = true;
                failedSnapshot.IsRefreshing = false;
                failedSnapshot.IsPaused = false;
                currentSnapshot = failedSnapshot;

                SetTrayIcon(IconRenderer.CreateErrorIcon());
                SetNotifyTooltip("");
                UpdateUsagePopup(failedSnapshot);
            }

            if (showBalloon)
            {
                notifyIcon.ShowBalloonTip(3000, "Bluetooth Battery", failureMessage, ToolTipIcon.Warning);
            }
        }

        private static string BuildPausedFailureStatus(string failureMessage)
        {
            if (string.IsNullOrWhiteSpace(failureMessage))
            {
                return PausedAfterFailureStatus;
            }

            return "Checks paused - " + failureMessage;
        }

        private static string GetFailureStatus(UsageSnapshot snapshot, string fallback)
        {
            return snapshot != null && !string.IsNullOrWhiteSpace(snapshot.ErrorMessage)
                ? snapshot.ErrorMessage
                : fallback;
        }

        private void ApplyPausedState()
        {
            refreshTimer.Stop();
            bool failedLastAttempt = currentSnapshot != null && currentSnapshot.IsStale;
            string failureMessage = failedLastAttempt && currentSnapshot != null
                ? currentSnapshot.ErrorMessage
                : null;

            if (lastSuccessfulSnapshot != null)
            {
                UsageSnapshot pausedSnapshot = lastSuccessfulSnapshot.Clone();
                pausedSnapshot.LastAttempted = lastAttemptedAt;
                pausedSnapshot.ErrorMessage = failureMessage;
                pausedSnapshot.StatusMessage = failedLastAttempt
                    ? BuildPausedFailureStatus(failureMessage)
                    : PausedStatus;
                pausedSnapshot.IsStale = failedLastAttempt;
                pausedSnapshot.IsRefreshing = refreshInProgress;
                pausedSnapshot.IsPaused = true;
                currentSnapshot = pausedSnapshot;

                SetNotifyTooltip(BuildNativeTooltip(pausedSnapshot));
                UpdateUsagePopup(pausedSnapshot);
            }
            else
            {
                UsageSnapshot waitingSnapshot = failedLastAttempt
                    ? currentSnapshot.Clone()
                    : UsageSnapshot.FromError(WaitingStatus);
                waitingSnapshot.LastUpdated = DateTime.MinValue;
                waitingSnapshot.LastAttempted = lastAttemptedAt;
                waitingSnapshot.ErrorMessage = failureMessage;
                waitingSnapshot.StatusMessage = failedLastAttempt
                    ? BuildPausedFailureStatus(failureMessage)
                    : WaitingStatus;
                waitingSnapshot.IsStale = failedLastAttempt;
                waitingSnapshot.IsRefreshing = refreshInProgress;
                waitingSnapshot.IsPaused = true;
                currentSnapshot = waitingSnapshot;

                SetTrayIcon(IconRenderer.CreateUnknownIcon("..."));
                SetNotifyTooltip("");
                UpdateUsagePopup(waitingSnapshot);
            }
            SetUsagePopupRefreshing(refreshInProgress);
        }

        private void RestoreActiveState()
        {
            if (currentSnapshot == null || !currentSnapshot.IsPaused)
            {
                return;
            }

            if (lastSuccessfulSnapshot != null)
            {
                UsageSnapshot activeSnapshot = currentSnapshot.IsStale
                    ? currentSnapshot.Clone()
                    : lastSuccessfulSnapshot.Clone();
                activeSnapshot.IsPaused = false;
                activeSnapshot.IsRefreshing = refreshInProgress;
                activeSnapshot.StatusMessage = refreshInProgress
                    ? RefreshingStatus
                    : (activeSnapshot.IsStale
                        ? GetFailureStatus(activeSnapshot, RefreshFailedStatus)
                        : "");
                currentSnapshot = activeSnapshot;
                RenderDataSnapshot(currentSnapshot, false, false, false);
            }
            else
            {
                UsageSnapshot activeSnapshot = currentSnapshot.Clone();
                activeSnapshot.IsPaused = false;
                activeSnapshot.IsRefreshing = refreshInProgress;
                activeSnapshot.StatusMessage = refreshInProgress
                    ? RefreshingStatus
                    : (activeSnapshot.IsStale
                        ? GetFailureStatus(activeSnapshot, RefreshFailedWithoutDataStatus)
                        : "Checking devices...");
                currentSnapshot = activeSnapshot;
                UpdateUsagePopup(currentSnapshot);
                SetUsagePopupRefreshing(refreshInProgress);
            }
        }

        private void RenderDataSnapshot(UsageSnapshot snapshot, bool renderIcon, bool updateThresholds, bool showBalloon)
        {
            if (snapshot == null)
            {
                return;
            }

            LimitWindow iconWindow = GetIconWindow(snapshot);
            if (renderIcon)
            {
                if (iconWindow == null)
                {
                    SetTrayIcon(IconRenderer.CreateUnknownIcon("..."));
                }
                else
                {
                    int remaining = ClampPercent(100.0 - iconWindow.UsedPercent);
                    SetTrayIcon(IconRenderer.CreatePercentIcon(remaining, settings));
                }
            }

            SetNotifyTooltip(BuildNativeTooltip(snapshot));
            UpdateUsagePopup(snapshot);
            bool thresholdBalloonShown = updateThresholds && UpdateThresholdNotifications(snapshot);

            if (showBalloon && !thresholdBalloonShown)
            {
                notifyIcon.ShowBalloonTip(2500, "Bluetooth Battery", BuildTooltip(snapshot), ToolTipIcon.Info);
            }
        }

        private static int ClampPercent(double value)
        {
            if (value < 0)
            {
                return 0;
            }
            if (value > 100)
            {
                return 100;
            }
            return (int)Math.Round(value);
        }

        private static string BuildTooltip(UsageSnapshot snapshot)
        {
            return FormatWindow(snapshot.Weekly) + " | " + FormatWindow(snapshot.FiveHour);
        }

        private static string BuildNativeTooltip(UsageSnapshot snapshot)
        {
            if (snapshot == null)
            {
                return "";
            }

            StringBuilder tooltip = new StringBuilder();
            if (snapshot.Devices != null && snapshot.Devices.Count > 0)
            {
                for (int index = 0; index < snapshot.Devices.Count; index++)
                {
                    AppendNativeTooltipLine(tooltip, snapshot.Devices[index]);
                }
            }
            else
            {
                AppendNativeTooltipLine(tooltip, snapshot.Weekly);
                AppendNativeTooltipLine(tooltip, snapshot.FiveHour);
            }
            return tooltip.ToString();
        }

        private static void AppendNativeTooltipLine(StringBuilder tooltip, LimitWindow window)
        {
            string line = FormatNativeWindow(window);
            if (string.IsNullOrEmpty(line))
            {
                return;
            }
            if (tooltip.Length > 0)
            {
                tooltip.Append(Environment.NewLine);
            }
            tooltip.Append(line);
        }

        private static string FormatWindow(LimitWindow window)
        {
            if (window == null)
            {
                return "No other supported device";
            }

            string name = string.IsNullOrWhiteSpace(window.DisplayName)
                ? "Wireless device"
                : window.DisplayName;
            string level = window.BatteryPercent.HasValue
                ? window.RemainingPercent + "%"
                : "unavailable";
            return name + ": " + level;
        }

        private static string FormatNativeWindow(LimitWindow window)
        {
            if (window == null)
            {
                return "";
            }

            string name = string.IsNullOrWhiteSpace(window.DisplayName)
                ? "Wireless device"
                : window.DisplayName;
            name = name.Replace('\r', ' ').Replace('\n', ' ').Trim();
            string level = window.BatteryPercent.HasValue
                ? window.BatteryPercent.Value.ToString()
                : "?";
            return name + " " + level + "%";
        }

        private static string TrimTooltip(string value)
        {
            if (string.IsNullOrEmpty(value))
            {
                return "";
            }

            return value.Length > 63 ? value.Substring(0, 63) : value;
        }

        private void SetNotifyTooltip(string value)
        {
            notifyIcon.Text = TrimTooltip(FirstTooltipLine(value));
            NativeTrayTooltip.TrySetText(notifyIcon, value);
        }

        private void UpdateUsagePopup(UsageSnapshot snapshot)
        {
            if (settingsForm != null && !settingsForm.IsDisposed && !settingsForm.Disposing)
            {
                settingsForm.UpdateDevices(snapshot);
            }
            UsagePopup popup = usagePopup;
            if (popup != null && !popup.IsDisposed && !popup.Disposing)
            {
                popup.UpdateSnapshot(snapshot);
            }
        }

        private void SetUsagePopupRefreshing(bool refreshing)
        {
            UsagePopup popup = usagePopup;
            if (popup != null && !popup.IsDisposed && !popup.Disposing)
            {
                popup.SetRefreshing(refreshing);
            }
        }

        private bool TryPostToUi(Action action)
        {
            if (shuttingDown || action == null || dispatcher.IsDisposed || !dispatcher.IsHandleCreated)
            {
                return false;
            }

            try
            {
                dispatcher.BeginInvoke(new Action(delegate
                {
                    if (!shuttingDown)
                    {
                        action();
                    }
                }));
                return true;
            }
            catch (ObjectDisposedException)
            {
                return false;
            }
            catch (InvalidOperationException)
            {
                return false;
            }
        }

        private static string FirstTooltipLine(string value)
        {
            if (string.IsNullOrEmpty(value))
            {
                return "";
            }

            int lineBreak = value.IndexOfAny(new char[] { '\r', '\n' });
            return lineBreak >= 0 ? value.Substring(0, lineBreak) : value;
        }

        private void SetTrayIcon(Icon icon)
        {
            Icon oldIcon = currentIcon;
            currentIcon = icon;
            notifyIcon.Icon = currentIcon;
            if (oldIcon != null)
            {
                oldIcon.Dispose();
            }
        }

        private LimitWindow GetIconWindow(UsageSnapshot snapshot)
        {
            if (snapshot == null)
            {
                return null;
            }

            if (string.Equals(settings.IconMetric, AppSettings.IconMetricDevice,
                StringComparison.OrdinalIgnoreCase))
            {
                if (snapshot.Devices != null && !string.IsNullOrEmpty(settings.IconDeviceId))
                {
                    foreach (LimitWindow device in snapshot.Devices)
                    {
                        if (device != null && device.BatteryPercent.HasValue &&
                            string.Equals(device.DeviceId, settings.IconDeviceId,
                                StringComparison.OrdinalIgnoreCase))
                        {
                            return device;
                        }
                    }
                }
                return null;
            }

            bool showAverage = string.Equals(
                settings.IconMetric,
                AppSettings.IconMetricFiveHour,
                StringComparison.OrdinalIgnoreCase);
            int? percent = showAverage
                ? snapshot.AverageBatteryPercent
                : snapshot.LowestBatteryPercent;
            if (!percent.HasValue)
            {
                return null;
            }

            LimitWindow aggregate = new LimitWindow();
            aggregate.BatteryPercent = percent;
            aggregate.UsedPercent = 100.0 - percent.Value;
            return aggregate;
        }

        private bool UpdateThresholdNotifications(UsageSnapshot snapshot)
        {
            int weeklyLevel = GetThresholdLevel(snapshot != null ? snapshot.Weekly : null);
            int fiveHourLevel = GetThresholdLevel(snapshot != null ? snapshot.FiveHour : null);

            bool shouldNotify = settings.ThresholdNotifications;
            bool weeklyAlert = shouldNotify && IsNewThresholdLevel(lastWeeklyThresholdLevel, weeklyLevel);
            bool fiveHourAlert = shouldNotify && IsNewThresholdLevel(lastFiveHourThresholdLevel, fiveHourLevel);

            lastWeeklyThresholdLevel = weeklyLevel;
            lastFiveHourThresholdLevel = fiveHourLevel;

            if (!weeklyAlert && !fiveHourAlert)
            {
                return false;
            }

            StringBuilder message = new StringBuilder();
            int highestLevel = 0;
            if (weeklyAlert)
            {
                AppendThresholdLine(message, snapshot.Weekly.DisplayName, snapshot.Weekly, weeklyLevel);
                highestLevel = Math.Max(highestLevel, weeklyLevel);
            }
            if (fiveHourAlert)
            {
                AppendThresholdLine(message, snapshot.FiveHour.DisplayName, snapshot.FiveHour, fiveHourLevel);
                highestLevel = Math.Max(highestLevel, fiveHourLevel);
            }

            notifyIcon.ShowBalloonTip(
                5000,
                highestLevel >= 2 ? "Battery Critical" : "Battery Low",
                message.ToString(),
                ToolTipIcon.Warning);
            return true;
        }

        private int GetThresholdLevel(LimitWindow window)
        {
            if (window == null || window.IsCached)
            {
                return 0;
            }

            int remaining = window.RemainingPercent;
            if (remaining <= settings.CriticalThreshold)
            {
                return 2;
            }
            if (remaining <= settings.LowThreshold)
            {
                return 1;
            }

            return 0;
        }

        private static bool IsNewThresholdLevel(int? previousLevel, int currentLevel)
        {
            return previousLevel.HasValue && currentLevel > previousLevel.Value;
        }

        private static void AppendThresholdLine(StringBuilder message, string label, LimitWindow window, int level)
        {
            if (message.Length > 0)
            {
                message.AppendLine();
            }

            string severity = level >= 2 ? "critical" : "low";
            int remaining = window != null ? window.RemainingPercent : 0;
            message.Append(label);
            message.Append(" battery is ");
            message.Append(severity);
            message.Append(": ");
            message.Append(remaining);
            message.Append("% remaining.");
        }

        private void ResetThresholdNotificationState()
        {
            lastWeeklyThresholdLevel = null;
            lastFiveHourThresholdLevel = null;
        }

        private void ToggleUsagePopup()
        {
            UsagePopup popup = usagePopup;
            if (popup != null && !popup.IsDisposed && popup.Visible)
            {
                popup.Hide();
            }
            else
            {
                ShowUsagePopup();
            }
        }

        private void ShowUsagePopup()
        {
            UsagePopup popup = EnsureUsagePopup();
            if (popup == null)
            {
                return;
            }

            UsageSnapshot snapshot = currentSnapshot;
            if (snapshot == null)
            {
                snapshot = UsageSnapshot.FromError("Checking devices...");
                snapshot.LastUpdated = DateTime.MinValue;
                snapshot.LastAttempted = lastAttemptedAt;
                snapshot.StatusMessage = "Checking devices...";
                snapshot.IsRefreshing = refreshInProgress;
            }

            popup.UpdateSnapshot(snapshot);
            popup.SetRefreshing(refreshInProgress);
            popup.ShowNear(Cursor.Position);
        }

        private void ShowSettings()
        {
            if (settingsForm != null && !settingsForm.IsDisposed)
            {
                ShowAndActivateSettingsForm();
                return;
            }

            settingsForm = new SettingsForm(settings, currentSnapshot);
            settingsForm.SetUpdateStatus(updateStatusText);
            settingsForm.SetUpdateButtonState(updateCheckInProgress || updateInstallInProgress,
                updateInstallInProgress ? "Installing..." : "Checking...");
            settingsForm.UpdateCheckRequested += delegate { CheckForUpdates(UpdateCheckMode.Manual); };
            SettingsForm openedSettingsForm = settingsForm;
            settingsForm.UnsavedChangesChanged += delegate { TryPostToUi(TryFinishPreparedUpdate); };
            settingsForm.FormClosed += delegate
            {
                if (ReferenceEquals(settingsForm, openedSettingsForm)) settingsForm = null;
                TryPostToUi(TryFinishPreparedUpdate);
            };
            settingsForm.SettingsApplied += delegate
            {
                ApplyStartupSetting();
                UsagePopup popup = usagePopup;
                if (popup != null && !popup.IsDisposed && !popup.Disposing)
                {
                    popup.ApplySettings(settings);
                }
                ResetThresholdNotificationState();
                if (currentSnapshot != null && lastSuccessfulSnapshot != null)
                {
                    RenderDataSnapshot(currentSnapshot, true, true, false);
                }
                RefreshScheduleChanged();
            };
            settingsForm.Show();
            ShowAndActivateSettingsForm();
        }

        private void ShowAndActivateSettingsForm()
        {
            if (settingsForm == null || settingsForm.IsDisposed)
            {
                return;
            }

            if (!settingsForm.Visible)
            {
                settingsForm.Show();
            }
            if (settingsForm.WindowState == FormWindowState.Minimized)
            {
                settingsForm.WindowState = FormWindowState.Normal;
            }

            IntPtr windowHandle = settingsForm.Handle;
            ShowWindow(windowHandle, ShowWindowRestore);
            settingsForm.BringToFront();
            settingsForm.Activate();
            SetForegroundWindow(windowHandle);
        }

        private void ApplyStartupSetting()
        {
            try
            {
                StartupManager.SetEnabled(settings.StartWithWindows);
            }
            catch (Exception ex)
            {
                settings.StartWithWindows = StartupManager.IsEnabled();
                string message = "Could not update Windows startup setting:" +
                    Environment.NewLine + Environment.NewLine + ex.Message;
                try
                {
                    settings.Save();
                }
                catch (Exception saveException)
                {
                    message += Environment.NewLine + Environment.NewLine +
                        "The corrected setting could not be saved: " + saveException.Message;
                }
                MessageBox.Show(
                    GetSettingsOwner(),
                    message,
                    "Bluetooth Battery Settings",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
            }
        }

        private UsagePopup CreateUsagePopup()
        {
            UsagePopup popup = new UsagePopup(settings);
            popup.RefreshRequested += delegate { RefreshUsage(false); };
            popup.SettingsRequested += delegate(object sender, EventArgs e)
            {
                UsagePopup source = sender as UsagePopup;
                if (source != null && !source.IsDisposed)
                {
                    source.Hide();
                }
                ShowSettings();
            };
            return popup;
        }

        private UsagePopup EnsureUsagePopup()
        {
            if (shuttingDown)
            {
                return null;
            }

            if (usagePopup == null || usagePopup.IsDisposed)
            {
                usagePopup = CreateUsagePopup();
            }
            return usagePopup;
        }

        private void CheckForUpdates(UpdateCheckMode mode)
        {
            if (shuttingDown)
            {
                return;
            }

            if (updateCheckInProgress || updateInstallInProgress)
            {
                if (mode == UpdateCheckMode.Manual)
                {
                    MessageBox.Show(GetSettingsOwner(), "An update check is already running.", "Bluetooth Battery Monitor Updates", MessageBoxButtons.OK, MessageBoxIcon.Information);
                }
                return;
            }

            updateCheckInProgress = true;
            SetSettingsUpdateButtonState(true, "Checking...");
            SetUpdateStatusText("Checking for updates...");

            CancellationToken cancellationToken = shutdownCancellation.Token;
            Task.Factory.StartNew(
                delegate
                {
                    cancellationToken.ThrowIfCancellationRequested();
                    return UpdateService.CheckForUpdate();
                },
                cancellationToken,
                TaskCreationOptions.None,
                TaskScheduler.Default).ContinueWith(delegate(Task<UpdateInfo> task)
            {
                if (task.IsFaulted)
                {
                    AggregateException ignored = task.Exception;
                }

                TryPostToUi(delegate
                {
                    updateCheckInProgress = false;
                    SetSettingsUpdateButtonState(false, null);
                    if (task.IsCanceled)
                    {
                        SetUpdateStatusText("Update check canceled.");
                    }
                    else if (task.IsFaulted)
                    {
                        HandleUpdateError(task.Exception, mode == UpdateCheckMode.Manual, false);
                    }
                    else
                    {
                        HandleUpdateInfo(task.Result, mode);
                    }
                });
            }, CancellationToken.None, TaskContinuationOptions.None, TaskScheduler.Default);
        }

        internal static UpdateCheckAction DecideUpdateAction(UpdateInfo info, UpdateCheckMode mode)
        {
            if (info == null || !info.UpdateAvailable) return UpdateCheckAction.None;
            if (mode != UpdateCheckMode.Manual && !info.CanInstall) return UpdateCheckAction.NotifyFailure;
            if (mode == UpdateCheckMode.Startup && info.CanInstall) return UpdateCheckAction.InstallAutomatically;
            if (mode != UpdateCheckMode.Manual) return UpdateCheckAction.None;
            return info.CanInstall ? UpdateCheckAction.PromptInstall : UpdateCheckAction.PromptRelease;
        }

        internal static bool TryMarkUninstallableUpdateNotice(UpdateInfo info, ISet<string> notices)
        {
            if (info == null || !info.UpdateAvailable || info.CanInstall || notices == null) return false;
            string key = (info.LatestVersion ?? "") + "\n" + (info.Message ?? "");
            return notices.Add(key);
        }

        private void HandleUpdateInfo(UpdateInfo info, UpdateCheckMode mode)
        {
            if (info == null)
            {
                SetUpdateStatusText("Could not read update information.");
                if (mode == UpdateCheckMode.Manual)
                {
                    MessageBox.Show(GetSettingsOwner(), "Could not read update information.", "Bluetooth Battery Monitor Updates", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                }
                return;
            }

            if (!info.UpdateAvailable)
            {
                SetUpdateStatusText(string.IsNullOrEmpty(info.LatestVersion) ? info.Message : "Up to date: " + info.LatestVersion);
                if (mode == UpdateCheckMode.Manual)
                {
                    MessageBox.Show(GetSettingsOwner(), info.Message, "Bluetooth Battery Monitor Updates", MessageBoxButtons.OK, MessageBoxIcon.Information);
                }
                return;
            }

            SetUpdateStatusText(info.CanInstall ? "Version " + info.LatestVersion + " available" : info.Message);

            UpdateCheckAction action = DecideUpdateAction(info, mode);
            if (action == UpdateCheckAction.InstallAutomatically)
            {
                InstallUpdate(info, false);
                return;
            }
            if (action == UpdateCheckAction.NotifyFailure)
            {
                if (TryMarkUninstallableUpdateNotice(info, uninstallableUpdateNotices))
                {
                    notifyIcon.ShowBalloonTip(5000, "Bluetooth Battery Monitor Update",
                        string.IsNullOrWhiteSpace(info.Message)
                            ? "The update could not be installed automatically. Open Settings for details."
                            : info.Message,
                        ToolTipIcon.Warning);
                }
                return;
            }
            if (action == UpdateCheckAction.None) return;

            if (action == UpdateCheckAction.PromptInstall)
            {
                DialogResult result = MessageBox.Show(
                    GetSettingsOwner(),
                    info.Message + Environment.NewLine + Environment.NewLine + "Install now? Bluetooth Battery Monitor will restart automatically.",
                    "Bluetooth Battery Monitor Updates",
                    MessageBoxButtons.YesNo,
                    MessageBoxIcon.Information);
                if (result == DialogResult.Yes)
                {
                    InstallUpdate(info, true);
                }
                return;
            }

            DialogResult openRelease = MessageBox.Show(
                GetSettingsOwner(),
                info.Message + Environment.NewLine + Environment.NewLine + "Open the release page?",
                "Bluetooth Battery Monitor Updates",
                MessageBoxButtons.YesNo,
                MessageBoxIcon.Information);
            if (openRelease == DialogResult.Yes)
            {
                OpenReleasePage();
            }
        }

        private void InstallUpdate(UpdateInfo info, bool interactive)
        {
            if (shuttingDown || updateInstallInProgress)
            {
                return;
            }

            updateInstallInProgress = true;
            preparedUpdateInteractive = interactive;
            SetSettingsUpdateButtonState(true, "Installing...");
            SetUpdateStatusText("Installing version " + info.LatestVersion + "...");

            CancellationToken cancellationToken = shutdownCancellation.Token;
            Task.Factory.StartNew(
                delegate
                {
                    cancellationToken.ThrowIfCancellationRequested();
                    return UpdateService.PrepareUpdate(info);
                },
                cancellationToken,
                TaskCreationOptions.None,
                TaskScheduler.Default).ContinueWith(delegate(Task<UpdateService.PreparedUpdate> task)
            {
                if (task.IsFaulted)
                {
                    AggregateException ignored = task.Exception;
                }

                if (task.Status == TaskStatus.RanToCompletion)
                {
                    lock (updatePreparationLock)
                    {
                        if (shuttingDown)
                        {
                            task.Result.Dispose();
                            return;
                        }
                        preparedUpdate = task.Result;
                    }
                }

                bool posted = TryPostToUi(delegate
                {
                    if (task.IsCanceled)
                    {
                        updateInstallInProgress = false;
                        SetSettingsUpdateButtonState(false, null);
                        SetUpdateStatusText("Update installation canceled.");
                        return;
                    }
                    if (task.IsFaulted)
                    {
                        updateInstallInProgress = false;
                        SetSettingsUpdateButtonState(false, null);
                        HandleUpdateError(task.Exception, interactive, true);
                        return;
                    }

                    TryFinishPreparedUpdate();
                });
                if (!posted && task.Status == TaskStatus.RanToCompletion)
                {
                    lock (updatePreparationLock)
                    {
                        if (ReferenceEquals(preparedUpdate, task.Result)) preparedUpdate = null;
                    }
                    task.Result.Dispose();
                }
            }, CancellationToken.None, TaskContinuationOptions.None, TaskScheduler.Default);
        }

        internal static bool TryHandoffWhenReady(Func<bool> hasUnsavedSettings, Func<bool> anotherInstanceRunning, Action handoff)
        {
            if (hasUnsavedSettings() || anotherInstanceRunning()) return false;
            handoff();
            return true;
        }

        private bool AnotherInstalledInstanceRunning()
        {
            Process[] processes;
            try
            {
                processes = Process.GetProcessesByName(Path.GetFileNameWithoutExtension(Application.ExecutablePath));
            }
            catch (Exception)
            {
                return true;
            }

            bool anotherInstance = false;
            int currentId = Process.GetCurrentProcess().Id;
            foreach (Process process in processes)
            {
                try
                {
                    if (process.Id != currentId && string.Equals(process.MainModule.FileName,
                        Application.ExecutablePath, StringComparison.OrdinalIgnoreCase)) anotherInstance = true;
                }
                catch (InvalidOperationException) { }
                catch (Exception) { anotherInstance = true; }
                finally { process.Dispose(); }
            }
            return anotherInstance;
        }

        private void TryFinishPreparedUpdate()
        {
            UpdateService.PreparedUpdate prepared;
            lock (updatePreparationLock) { prepared = preparedUpdate; }
            if (shuttingDown || prepared == null) return;

            bool unsaved = false;
            try
            {
                // Settings cannot change on this UI thread between the final check and exit.
                bool started = TryHandoffWhenReady(
                    delegate { unsaved = settingsForm != null && !settingsForm.IsDisposed && settingsForm.HasUnsavedChanges; return unsaved; },
                    AnotherInstalledInstanceRunning,
                    prepared.Start);
                if (!started)
                {
                    SetUpdateStatusText(unsaved
                        ? "Update ready; waiting for unsaved settings to be saved or closed."
                        : "Update ready; waiting for other running instances to close.");
                    if (updateWaitTimer == null)
                    {
                        updateWaitTimer = new System.Windows.Forms.Timer();
                        updateWaitTimer.Interval = 2000;
                        updateWaitTimer.Tick += delegate { TryFinishPreparedUpdate(); };
                    }
                    updateWaitTimer.Start();
                    return;
                }
                if (updateWaitTimer != null) updateWaitTimer.Stop();
                lock (updatePreparationLock) { preparedUpdate = null; }
                ExitThread();
            }
            catch (Exception ex)
            {
                lock (updatePreparationLock) { preparedUpdate = null; }
                prepared.Dispose();
                updateInstallInProgress = false;
                SetSettingsUpdateButtonState(false, null);
                HandleUpdateError(new AggregateException(ex), preparedUpdateInteractive, true);
            }
        }

        private void HandleUpdateError(AggregateException exception, bool interactive, bool installing)
        {
            Exception baseException = exception != null ? exception.GetBaseException() : null;
            string message = baseException != null ? baseException.Message : "Unknown update error.";
            SetUpdateStatusText((installing ? "Update installation failed: " : "Update check failed: ") + message);

            if (!interactive)
            {
                if (installing)
                {
                    notifyIcon.ShowBalloonTip(5000, "Bluetooth Battery Monitor Update",
                        "Automatic update failed. Open Settings and check for updates to retry.", ToolTipIcon.Warning);
                }
                return;
            }

            MessageBox.Show(GetSettingsOwner(), (installing ? "Could not install the update:" : "Could not check for updates:") + Environment.NewLine + Environment.NewLine + message, "Bluetooth Battery Monitor Updates", MessageBoxButtons.OK, MessageBoxIcon.Warning);
        }

        private IWin32Window GetSettingsOwner()
        {
            return settingsForm != null && !settingsForm.IsDisposed ? settingsForm : null;
        }

        private void SetSettingsUpdateButtonState(bool busy, string busyText)
        {
            if (settingsForm != null && !settingsForm.IsDisposed)
            {
                settingsForm.SetUpdateButtonState(busy, busyText);
            }
        }

        private void SetUpdateStatusText(string status)
        {
            updateStatusText = status ?? "";
            if (settingsForm != null && !settingsForm.IsDisposed)
            {
                settingsForm.SetUpdateStatus(updateStatusText);
            }
        }

        private void OpenReleasePage()
        {
            try
            {
                ProcessStartInfo startInfo = new ProcessStartInfo(AppVersion.GitHubRepoUrl + "/releases/latest");
                startInfo.UseShellExecute = true;
                Process.Start(startInfo);
            }
            catch (Exception ex)
            {
                MessageBox.Show(GetSettingsOwner(), "Could not open release page:" + Environment.NewLine + Environment.NewLine + ex.Message, "Bluetooth Battery Monitor Updates", MessageBoxButtons.OK, MessageBoxIcon.Warning);
            }
        }

        protected override void ExitThreadCore()
        {
            if (shuttingDown)
            {
                return;
            }

            shuttingDown = true;
            SystemEvents.PowerModeChanged -= OnPowerModeChanged;
            if (connectionWatcher != null)
            {
                connectionWatcher.Dispose();
                connectionWatcher = null;
            }
            connectionDebounceTimer.Stop();
            connectionDebounceTimer.Dispose();
            try
            {
                shutdownCancellation.Cancel();
            }
            catch (ObjectDisposedException)
            {
            }
            if (updateWaitTimer != null)
            {
                updateWaitTimer.Stop();
                updateWaitTimer.Dispose();
                updateWaitTimer = null;
            }
            lock (updatePreparationLock)
            {
                if (preparedUpdate != null)
                {
                    preparedUpdate.Dispose();
                    preparedUpdate = null;
                }
            }
            PersistLearnedBatteryRates(null);
            PersistBatteryCache(null);

            refreshTimer.Stop();
            refreshTimer.Dispose();
            updateTimer.Stop();
            updateTimer.Dispose();
            activityTimer.Stop();
            activityTimer.Dispose();
            if (startupUiTimer != null)
            {
                startupUiTimer.Stop();
                startupUiTimer.Dispose();
                startupUiTimer = null;
            }
            if (usagePopup != null && !usagePopup.IsDisposed)
            {
                usagePopup.Dispose();
                usagePopup = null;
            }
            if (settingsForm != null && !settingsForm.IsDisposed)
            {
                settingsForm.Dispose();
                settingsForm = null;
            }
            notifyIcon.Visible = false;
            notifyIcon.Dispose();
            if (currentIcon != null)
            {
                currentIcon.Dispose();
                currentIcon = null;
            }
            dispatcher.Dispose();
            refreshClock.Stop();
            shutdownCancellation.Dispose();
            base.ExitThreadCore();
        }
    }
}
