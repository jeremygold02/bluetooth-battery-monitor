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
            string argsPath = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "fixture-args.txt");
            File.WriteAllLines(argsPath + ".tmp", Environment.GetCommandLineArgs());
            if (File.Exists(argsPath)) File.Delete(argsPath);
            File.Move(argsPath + ".tmp", argsPath);
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
            TestUpdateDecisions();
            TestReleaseVersionValidation(directory);
            TestReplacement(directory, true);
            TestReplacement(directory, false);
            TestExistingBackup(directory);
            TestConcurrentUpdates(directory);
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

    private static void TestUpdateDecisions()
    {
        UpdateInfo available = new UpdateInfo { UpdateAvailable = true, CanInstall = true };
        UpdateInfo unavailable = new UpdateInfo { UpdateAvailable = true, CanInstall = false };
        UpdateInfo current = new UpdateInfo { UpdateAvailable = false, CanInstall = false };
        Assert(TrayAppContext.DecideUpdateAction(available, TrayAppContext.UpdateCheckMode.Startup) == TrayAppContext.UpdateCheckAction.InstallAutomatically, "startup must install an available verified update");
        Assert(TrayAppContext.DecideUpdateAction(unavailable, TrayAppContext.UpdateCheckMode.Startup) == TrayAppContext.UpdateCheckAction.Notify, "startup must notify when automatic installation is unavailable");
        Assert(TrayAppContext.DecideUpdateAction(available, TrayAppContext.UpdateCheckMode.Periodic) == TrayAppContext.UpdateCheckAction.Notify, "periodic checks must notify instead of restarting unexpectedly");
        Assert(TrayAppContext.DecideUpdateAction(available, TrayAppContext.UpdateCheckMode.Manual) == TrayAppContext.UpdateCheckAction.PromptInstall, "manual checks must offer installation");
        Assert(TrayAppContext.DecideUpdateAction(unavailable, TrayAppContext.UpdateCheckMode.Manual) == TrayAppContext.UpdateCheckAction.PromptRelease, "manual checks must offer the release page when installation is unavailable");
        Assert(TrayAppContext.DecideUpdateAction(current, TrayAppContext.UpdateCheckMode.Startup) == TrayAppContext.UpdateCheckAction.None, "the current version must not start installation");
        Assert(TrayAppContext.DecideUpdateAction(null, TrayAppContext.UpdateCheckMode.Startup) == TrayAppContext.UpdateCheckAction.None, "missing update information must not start installation");
        Assert(!TrayAppContext.ShouldCheckForStartupUpdate(new string[] { "--skip-startup-update", "--update-failed" }), "updater restart must skip the startup check");
        Assert(TrayAppContext.ShouldCheckForStartupUpdate(new string[] { "--show-settings" }), "ordinary launches must check for updates");
        Assert(TrayAppContext.ShouldCheckForStartupUpdate(null), "launches without arguments must check for updates");
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

    private static void TestReleaseVersionValidation(string directory)
    {
        string parsed;
        Assert(UpdateService.TryParseReleaseVersion("0.1.2", out parsed) && parsed == "0.1.2", "valid tag rejected");
        Assert(UpdateService.TryParseReleaseVersion("v0.1.2", out parsed) && parsed == "0.1.2", "v-prefixed tag rejected");
        foreach (string invalid in new string[] { "", "invalid", "release-0.1.2", "1.2", "1.2.3-beta", "01.2.3", "1.2.2147483648" })
            Assert(!UpdateService.TryParseReleaseVersion(invalid, out parsed), "malformed release tag accepted: " + invalid);

        string sentinel = Path.Combine(directory, "untrusted-code-ran.txt");
        string valid = CompileMetadataFixture(directory, "valid", "BluetoothBatteryMonitor.exe", "1.2.3", "jeremygold02", "bluetooth-battery-monitor", sentinel);
        string wrongOwner = CompileMetadataFixture(directory, "wrong-owner", "BluetoothBatteryMonitor.exe", "1.2.3", "other", "bluetooth-battery-monitor", sentinel);
        string wrongIdentity = CompileMetadataFixture(directory, "wrong-identity", "OtherApplication.exe", "1.2.3", "jeremygold02", "bluetooth-battery-monitor", sentinel);
        UpdateService.VerifyManagedRelease(valid, "1.2.3");
        AssertMetadataRejected(valid, "1.2.4");
        AssertMetadataRejected(valid, "release-1.2.3");
        AssertMetadataRejected(wrongOwner, "1.2.3");
        AssertMetadataRejected(wrongIdentity, "1.2.3");
        string malformed = Path.Combine(directory, "malformed.exe");
        File.WriteAllText(malformed, "MZ but not a managed executable");
        AssertMetadataRejected(malformed, "1.2.3");
        Assert(!File.Exists(sentinel), "inspecting update metadata executed untrusted code");
        File.Delete(valid);
        Assert(!File.Exists(valid), "metadata inspection left the staged executable locked");
    }

    private static void AssertMetadataRejected(string file, string version)
    {
        try { UpdateService.VerifyManagedRelease(file, version); }
        catch (UpdateException) { return; }
        throw new Exception("invalid release identity or version was accepted");
    }

    private static string CompileMetadataFixture(string root, string name, string executableName, string version, string owner, string repo, string sentinel)
    {
        string directory = Path.Combine(root, name);
        Directory.CreateDirectory(directory);
        string source = Path.Combine(directory, "fixture.cs");
        string output = Path.Combine(directory, executableName);
        string marker = sentinel.Replace("\\", "\\\\").Replace("\"", "\\\"");
        File.WriteAllText(source,
            "using System.IO; namespace CodexUsageTray { internal static class BuildVersion { " +
            "public const string Version = \"" + version + "\"; public const string GitHubOwner = \"" + owner +
            "\"; public const string GitHubRepo = \"" + repo + "\"; static BuildVersion() { File.WriteAllText(\"" + marker + "\", \"executed\"); } } " +
            "internal class Program { static void Main() { File.WriteAllText(\"" + marker + "\", \"executed\"); } } }");
        string windows = Environment.GetFolderPath(Environment.SpecialFolder.Windows);
        string compiler = Path.Combine(windows, @"Microsoft.NET\Framework64\v4.0.30319\csc.exe");
        if (!File.Exists(compiler)) compiler = Path.Combine(windows, @"Microsoft.NET\Framework\v4.0.30319\csc.exe");
        ProcessStartInfo start = new ProcessStartInfo(compiler, "/nologo /target:winexe /out:\"" + output + "\" \"" + source + "\"");
        start.UseShellExecute = false;
        start.CreateNoWindow = true;
        using (Process process = Process.Start(start))
        {
            process.WaitForExit();
            Assert(process.ExitCode == 0, "could not compile metadata fixture");
        }
        return output;
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
            Assert(exitCode == (success ? 0 : 1), "unexpected updater result: " + exitCode);
            Assert(Hash(target) == (success ? replacementHash : originalHash), "installed executable did not match expected replacement or rollback");
            Assert(File.ReadAllText(unrelated) == "unrelated user file", "existing .bak file was modified");
            Assert(File.Exists(backup) != success, "backup cleanup or failure retention was incorrect");
            AssertRestartArguments(appDir, success);
        }
        catch (Exception ex)
        {
            throw new Exception("Replacement fixture failed. Updater log: " + ReadUpdaterLog(jobDir), ex);
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
            AssertRestartArguments(appDir, false);
        }
        finally
        {
            StopFixture(appDir, target);
        }
    }

    private static void AssertRestartArguments(string directory, bool success)
    {
        string arguments = Path.Combine(directory, "fixture-args.txt");
        for (int attempt = 0; attempt < 100 && !File.Exists(arguments); attempt++) Thread.Sleep(50);
        Assert(File.Exists(arguments), "restarted app did not record its arguments");
        string[] values = File.ReadAllLines(arguments);
        Assert(Array.IndexOf(values, "--skip-startup-update") >= 0, "updater restart must skip one startup check");
        Assert(Array.IndexOf(values, success ? "--update-complete" : "--update-failed") >= 0, "updater restart must report its outcome");
    }

    private static void TestConcurrentUpdates(string root)
    {
        string caseDir = Path.Combine(root, "concurrent");
        string appDir = Path.Combine(caseDir, "app");
        string firstJob = Path.Combine(caseDir, "job-one");
        string secondJob = Path.Combine(caseDir, "job-two");
        Directory.CreateDirectory(appDir);
        Directory.CreateDirectory(firstJob);
        Directory.CreateDirectory(secondJob);
        string target = Path.Combine(appDir, "fixture.exe");
        string firstStaged = Path.Combine(appDir, ".first.tmp");
        string secondStaged = Path.Combine(appDir, ".second.tmp");
        File.Copy(Assembly.GetExecutingAssembly().Location, target);
        File.Copy(target, firstStaged);
        File.Copy(target, secondStaged);
        using (FileStream file = new FileStream(firstStaged, FileMode.Append, FileAccess.Write)) file.WriteByte(41);
        using (FileStream file = new FileStream(secondStaged, FileMode.Append, FileAccess.Write)) file.WriteByte(42);
        string firstHash = Hash(firstStaged);
        string secondHash = Hash(secondStaged);
        File.WriteAllText(Path.Combine(appDir, "fixture-mode.txt"), "sleep");
        string firstScript = WriteScript(firstJob, target, firstStaged, firstStaged + ".bak");
        string secondScript = WriteScript(secondJob, target, secondStaged, secondStaged + ".bak");
        try
        {
            using (Process first = StartScript(firstScript))
            using (Process second = StartScript(secondScript))
            {
                bool firstExited = first.WaitForExit(45000);
                bool secondExited = second.WaitForExit(45000);
                if (!firstExited || !secondExited)
                {
                    if (!firstExited) first.Kill();
                    if (!secondExited) second.Kill();
                    throw new Exception("Concurrent updater scripts timed out.");
                }
                Assert((first.ExitCode == 0 && second.ExitCode == 1) || (first.ExitCode == 1 && second.ExitCode == 0), "exactly one concurrent update must succeed");
                Assert(Hash(target) == (first.ExitCode == 0 ? firstHash : secondHash), "a stale concurrent update replaced the winning executable");
                string marker = Path.Combine(appDir, "fixture.pid");
                Assert(File.Exists(marker), "winning update did not start the app");
                using (Process winner = Process.GetProcessById(int.Parse(File.ReadAllText(marker))))
                    Assert(!winner.HasExited, "stale update stopped the newly installed app");
            }
        }
        finally { StopFixture(appDir, target); }
    }

    private static int RunScript(string jobDir, string target, string staged, string backup)
    {
        string script = WriteScript(jobDir, target, staged, backup);
        using (Process process = StartScript(script))
        {
            if (!process.WaitForExit(45000))
            {
                process.Kill();
                throw new Exception("Isolated updater test timed out.");
            }
            return process.ExitCode;
        }
    }

    private static string WriteScript(string jobDir, string target, string staged, string backup)
    {
        string script = Path.Combine(jobDir, "update.ps1");
        typeof(UpdateService).GetMethod("WriteUpdaterScript", BindingFlags.Static | BindingFlags.NonPublic)
            .Invoke(null, new object[] { script, staged, target, backup, int.MaxValue });
        return script;
    }

    private static string ReadUpdaterLog(string jobDir)
    {
        string log = Path.Combine(jobDir, "update.log");
        try { return File.Exists(log) ? File.ReadAllText(log) : "(no updater log was written)"; }
        catch (IOException ex) { return "(could not read updater log: " + ex.Message + ")"; }
    }

    private static Process StartScript(string script)
    {
        string powershell = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), @"WindowsPowerShell\v1.0\powershell.exe");
        ProcessStartInfo start = new ProcessStartInfo(powershell, "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"" + script + "\"");
        start.UseShellExecute = false;
        start.CreateNoWindow = true;
        return Process.Start(start);
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
