using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using CodexUsageTray;

internal static class UpdaterTests
{
    private static void Main()
    {
        string marker = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "fixture-mode.txt");
        if (string.Equals(Path.GetFileName(Assembly.GetExecutingAssembly().Location), "fixture.exe", StringComparison.OrdinalIgnoreCase))
        {
            if (File.Exists(marker) && File.ReadAllText(marker) == "sleep")
            {
                string pidPath = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "fixture.pid");
                File.WriteAllText(pidPath + ".tmp", Process.GetCurrentProcess().Id.ToString());
                File.Move(pidPath + ".tmp", pidPath);
                Thread.Sleep(20000);
            }
            return;
        }

        RunTests();
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static void RunTests()
    {
        string tempRoot = Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar);
        string directory = Path.GetFullPath(Path.Combine(tempRoot, "BluetoothUpdaterTests-" + Guid.NewGuid().ToString("N")));
        if (!string.Equals(Path.GetDirectoryName(directory), tempRoot, StringComparison.OrdinalIgnoreCase))
            throw new Exception("Test directory is outside the temporary folder.");
        Directory.CreateDirectory(directory);
        try
        {
            TestAssetVerification(directory);
            TestReplacement(directory, true);
            TestReplacement(directory, false);
            TestExistingBackup(directory);
            Console.WriteLine("Updater verification and isolated replacement tests passed.");
        }
        finally
        {
            for (int attempt = 0; ; attempt++)
            {
                try { Directory.Delete(directory, true); break; }
                catch (IOException) { if (attempt >= 50) throw; Thread.Sleep(100); }
                catch (UnauthorizedAccessException) { if (attempt >= 50) throw; Thread.Sleep(100); }
            }
        }
    }

    private static void TestAssetVerification(string directory)
    {
        string file = Path.Combine(directory, "asset.bin");
        File.WriteAllBytes(file, Encoding.UTF8.GetBytes("verified asset"));
        string hash = Hash(file);
        InvokeVerify(file, new FileInfo(file).Length, hash, true);
        InvokeVerify(file, new FileInfo(file).Length + 1, hash, false);
        InvokeVerify(file, new FileInfo(file).Length, new string('0', 64), false);
    }

    private static void InvokeVerify(string file, long size, string hash, bool expectedSuccess)
    {
        MethodInfo verify = typeof(UpdateService).GetMethod("VerifyAssetFile", BindingFlags.Static | BindingFlags.NonPublic);
        try
        {
            verify.Invoke(null, new object[] { file, size, hash });
            Assert(expectedSuccess, "incorrect metadata was accepted");
        }
        catch (TargetInvocationException ex)
        {
            Assert(!expectedSuccess && ex.InnerException is UpdateException, "valid asset rejected or unexpected verification failure");
        }
    }

    private static void TestReplacement(string root, bool success)
    {
        string caseDir = Path.Combine(root, success ? "success" : "rollback");
        string appDir = Path.Combine(caseDir, "app");
        string jobDir = Path.Combine(caseDir, "job");
        Directory.CreateDirectory(appDir);
        Directory.CreateDirectory(jobDir);
        string target = Path.Combine(appDir, "fixture.exe");
        string staged = Path.Combine(appDir, ".fixture.update.tmp");
        string backup = staged + ".bak";
        string unrelated = target + ".bak";
        File.Copy(Assembly.GetExecutingAssembly().Location, target);
        File.Copy(target, staged);
        using (FileStream file = new FileStream(staged, FileMode.Append, FileAccess.Write)) file.WriteByte(42);
        string originalHash = Hash(target);
        string replacementHash = Hash(staged);
        File.WriteAllText(unrelated, "unrelated user file");
        File.WriteAllText(Path.Combine(appDir, "fixture-mode.txt"), success ? "sleep" : "exit");
        try
        {
            int exitCode = RunScript(jobDir, target, staged, backup);
            Assert(exitCode == (success ? 0 : 1), "unexpected updater result");
            Assert(Hash(target) == (success ? replacementHash : originalHash), "installed executable did not match expected replacement or rollback");
            Assert(File.ReadAllText(unrelated) == "unrelated user file", "existing .bak file was modified");
            Assert(File.Exists(backup) != success, "backup cleanup or failure retention was incorrect");
        }
        finally
        {
            StopFixture(appDir, target);
        }
    }

    private static void TestExistingBackup(string root)
    {
        string caseDir = Path.Combine(root, "collision");
        string appDir = Path.Combine(caseDir, "app");
        string jobDir = Path.Combine(caseDir, "job");
        Directory.CreateDirectory(appDir);
        Directory.CreateDirectory(jobDir);
        string target = Path.Combine(appDir, "fixture.exe");
        string staged = Path.Combine(appDir, ".fixture.update.tmp");
        string backup = staged + ".bak";
        File.Copy(Assembly.GetExecutingAssembly().Location, target);
        File.Copy(target, staged);
        File.WriteAllText(backup, "existing file");
        File.WriteAllText(Path.Combine(appDir, "fixture-mode.txt"), "sleep");
        string originalHash = Hash(target);
        try
        {
            Assert(RunScript(jobDir, target, staged, backup) == 1, "preexisting update backup must stop replacement");
            Assert(Hash(target) == originalHash && File.ReadAllText(backup) == "existing file", "preexisting update backup was changed");
        }
        finally
        {
            StopFixture(appDir, target);
        }
    }

    private static int RunScript(string jobDir, string target, string staged, string backup)
    {
        string script = Path.Combine(jobDir, "update.ps1");
        typeof(UpdateService).GetMethod("WriteUpdaterScript", BindingFlags.Static | BindingFlags.NonPublic)
            .Invoke(null, new object[] { script, staged, target, backup, int.MaxValue });
        string powershell = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), @"WindowsPowerShell\v1.0\powershell.exe");
        ProcessStartInfo start = new ProcessStartInfo(powershell, "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"" + script + "\"");
        start.UseShellExecute = false;
        start.CreateNoWindow = true;
        using (Process process = Process.Start(start))
        {
            if (!process.WaitForExit(45000))
            {
                process.Kill();
                throw new Exception("Isolated updater test timed out.");
            }
            return process.ExitCode;
        }
    }

    private static void StopFixture(string directory, string target)
    {
        string marker = Path.Combine(directory, "fixture.pid");
        string mode = Path.Combine(directory, "fixture-mode.txt");
        if (File.Exists(mode) && File.ReadAllText(mode) == "sleep")
        {
            for (int attempt = 0; attempt < 100 && !File.Exists(marker); attempt++) Thread.Sleep(50);
        }
        int pid;
        if (!File.Exists(marker) || !int.TryParse(File.ReadAllText(marker), out pid)) return;
        try
        {
            using (Process process = Process.GetProcessById(pid))
            {
                if (string.Equals(process.MainModule.FileName, target, StringComparison.OrdinalIgnoreCase))
                {
                    process.Kill();
                    process.WaitForExit();
                }
            }
        }
        catch (ArgumentException) { }
        catch (InvalidOperationException) { }
    }

    private static string Hash(string path)
    {
        using (SHA256 hash = SHA256.Create())
        using (FileStream file = File.OpenRead(path))
            return BitConverter.ToString(hash.ComputeHash(file)).Replace("-", "").ToLowerInvariant();
    }

    private static void Assert(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }
}
