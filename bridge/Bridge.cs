using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace CodexUsageTray
{
    internal static class BridgeProgram
    {
        private const string PayloadName = "BluetoothBatteryMonitor.Native.exe";
        private const string BridgeMutexName = "BluetoothBatteryMonitor.Bridge";
        private const string UpdateCompleteArgument = "--update-complete";
        private const string SkipStartupUpdateArgument = "--skip-startup-update";
        private const int MutexWaitMilliseconds = 15000;
        private const int StartupWatchMilliseconds = 4000;
        private const int UpdaterGraceMilliseconds = 9000;
        private const int RestoreAttempts = 20;
        private const int UseShowWindowFlag = 0x1;
        private const short HiddenWindow = 0;

        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfo
        {
            public int Size;
            public IntPtr Reserved;
            public IntPtr Desktop;
            public IntPtr Title;
            public int X;
            public int Y;
            public int Width;
            public int Height;
            public int ColumnCount;
            public int RowCount;
            public int FillAttribute;
            public int Flags;
            public short ShowWindow;
            public short ReservedSize;
            public IntPtr ReservedData;
            public IntPtr StandardInput;
            public IntPtr StandardOutput;
            public IntPtr StandardError;
        }

        [DllImport("kernel32.dll", EntryPoint = "GetStartupInfoW")]
        private static extern void GetStartupInfo(out StartupInfo startupInfo);

        [STAThread]
        private static int Main(string[] args)
        {
            bool announced = HasArgument(args, UpdateCompleteArgument);
            bool fromUpdater = announced || StartedHidden();
            if (fromUpdater && !announced)
            {
                List<string> extended = new List<string>(args);
                extended.Add(SkipStartupUpdateArgument);
                extended.Add(UpdateCompleteArgument);
                args = extended.ToArray();
            }
            Stopwatch running = Stopwatch.StartNew();
            try
            {
                Install(args, fromUpdater);
                if (fromUpdater)
                {
                    int remaining = UpdaterGraceMilliseconds - (int)running.ElapsedMilliseconds;
                    if (remaining > 0)
                    {
                        Thread.Sleep(remaining);
                    }
                }
                return 0;
            }
            catch (Exception ex)
            {
                if (!fromUpdater)
                {
                    MessageBox.Show(
                        "Bluetooth Battery Monitor could not finish updating:" + Environment.NewLine + Environment.NewLine + ex.Message,
                        "Bluetooth Battery Monitor",
                        MessageBoxButtons.OK,
                        MessageBoxIcon.Warning);
                }
                return 1;
            }
        }

        private static bool StartedHidden()
        {
            StartupInfo startupInfo;
            GetStartupInfo(out startupInfo);
            return (startupInfo.Flags & UseShowWindowFlag) != 0 && startupInfo.ShowWindow == HiddenWindow;
        }

        private static void Install(string[] args, bool fromUpdater)
        {
            string self = Path.GetFullPath(Assembly.GetExecutingAssembly().Location);
            string directory = Path.GetDirectoryName(self);
            string name = Path.GetFileName(self);
            if (string.IsNullOrEmpty(directory) || string.IsNullOrEmpty(name))
            {
                throw new InvalidOperationException("The app location could not be determined.");
            }

            using (Mutex mutex = new Mutex(false, BridgeMutexName))
            {
                bool owned;
                try
                {
                    owned = mutex.WaitOne(MutexWaitMilliseconds);
                }
                catch (AbandonedMutexException)
                {
                    owned = true;
                }
                if (!owned)
                {
                    throw new InvalidOperationException("Another update is still in progress.");
                }

                try
                {
                    string payloadHash = HashPayload();
                    if (File.Exists(self) && string.Equals(HashFile(self), payloadHash, StringComparison.Ordinal))
                    {
                        return;
                    }
                    Replace(self, directory, name, payloadHash, args, fromUpdater);
                }
                finally
                {
                    mutex.ReleaseMutex();
                }
            }
        }

        private static void Replace(string self, string directory, string name, string payloadHash, string[] args, bool fromUpdater)
        {
            string staged = Path.Combine(directory, "." + name + ".update-" + Guid.NewGuid().ToString("N") + ".tmp");
            string retired = staged + ".bak";
            try
            {
                WritePayload(staged);
                if (!string.Equals(HashFile(staged), payloadHash, StringComparison.Ordinal))
                {
                    throw new IOException("The app file could not be written correctly.");
                }

                File.Move(self, retired);
                try
                {
                    File.Move(staged, self);
                }
                catch
                {
                    File.Move(retired, self);
                    throw;
                }

                Process started;
                try
                {
                    started = Start(self, directory, args);
                }
                catch
                {
                    Restore(self, retired);
                    throw;
                }

                using (started)
                {
                    bool exited = started.WaitForExit(StartupWatchMilliseconds);
                    if (exited && (fromUpdater || started.ExitCode != 0))
                    {
                        Restore(self, retired);
                        throw new InvalidOperationException("The updated app did not stay running.");
                    }
                }
            }
            finally
            {
                TryDelete(staged);
            }
        }

        private static Process Start(string executable, string directory, string[] args)
        {
            StringBuilder arguments = new StringBuilder();
            foreach (string argument in args)
            {
                if (arguments.Length > 0)
                {
                    arguments.Append(' ');
                }
                arguments.Append('"').Append((argument ?? "").Replace("\"", "\\\"")).Append('"');
            }

            ProcessStartInfo startInfo = new ProcessStartInfo(executable, arguments.ToString());
            startInfo.UseShellExecute = false;
            startInfo.CreateNoWindow = true;
            startInfo.WorkingDirectory = directory;
            Process started = Process.Start(startInfo);
            if (started == null)
            {
                throw new InvalidOperationException("The updated app could not be started.");
            }
            return started;
        }

        private static void Restore(string self, string retired)
        {
            for (int attempt = 1; attempt <= RestoreAttempts; attempt++)
            {
                try
                {
                    if (File.Exists(self))
                    {
                        File.Delete(self);
                    }
                    File.Move(retired, self);
                    return;
                }
                catch (IOException)
                {
                    Thread.Sleep(250);
                }
                catch (UnauthorizedAccessException)
                {
                    Thread.Sleep(250);
                }
            }
        }

        private static Stream OpenPayload()
        {
            Stream payload = Assembly.GetExecutingAssembly().GetManifestResourceStream(PayloadName);
            if (payload == null)
            {
                throw new InvalidOperationException("This update does not contain the app.");
            }
            return payload;
        }

        private static string HashPayload()
        {
            using (Stream payload = OpenPayload())
            using (SHA256 sha256 = SHA256.Create())
            {
                return BitConverter.ToString(sha256.ComputeHash(payload));
            }
        }

        private static string HashFile(string path)
        {
            using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read | FileShare.Delete))
            using (SHA256 sha256 = SHA256.Create())
            {
                return BitConverter.ToString(sha256.ComputeHash(stream));
            }
        }

        private static void WritePayload(string path)
        {
            using (Stream payload = OpenPayload())
            using (FileStream destination = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None))
            {
                payload.CopyTo(destination);
                destination.Flush(true);
            }
        }

        private static void TryDelete(string path)
        {
            try
            {
                if (File.Exists(path))
                {
                    File.Delete(path);
                }
            }
            catch (IOException)
            {
            }
            catch (UnauthorizedAccessException)
            {
            }
        }

        private static bool HasArgument(string[] args, string value)
        {
            foreach (string argument in args)
            {
                if (string.Equals(argument, value, StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }
            return false;
        }
    }
}
