using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Threading;
using System.Windows.Forms;

namespace CodexUsageTray
{
    internal static class LegacyHost
    {
        private const BindingFlags Hidden = BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public;

        private static int Main(string[] args)
        {
            try
            {
                if (args.Length == 0 || HasArgument(args, "--update-failed"))
                {
                    File.WriteAllText(Application.ExecutablePath + ".restored", "restarted " + string.Join(" ", args));
                    Thread.Sleep(8000);
                    return 0;
                }
                if (args.Length >= 2 && args[0] == "--select")
                {
                    Console.WriteLine(SelectAsset(args));
                    return 0;
                }
                if (args.Length == 3 && args[0] == "--install")
                {
                    Install(Path.GetFullPath(args[1]), args[2]);
                    return 0;
                }
                Console.Error.WriteLine("Unknown arguments.");
                return 2;
            }
            catch (TargetInvocationException ex)
            {
                Console.Error.WriteLine((ex.InnerException ?? ex).Message);
                return 1;
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(ex.Message);
                return 1;
            }
        }

        private static bool Has(string name)
        {
            return typeof(UpdateService).GetMethod(name, Hidden) != null;
        }

        private static object Invoke(string name, params object[] arguments)
        {
            MethodInfo method = typeof(UpdateService).GetMethod(name, Hidden);
            if (method == null)
            {
                throw new MissingMethodException("UpdateService." + name);
            }
            return method.Invoke(null, arguments);
        }

        private static string SelectAsset(string[] args)
        {
            ArrayList assets = new ArrayList();
            for (int index = 1; index < args.Length; index++)
            {
                Dictionary<string, object> asset = new Dictionary<string, object>();
                asset["name"] = args[index];
                assets.Add(asset);
            }
            Dictionary<string, object> release = new Dictionary<string, object>();
            release["assets"] = assets;
            Dictionary<string, object> selected = (Dictionary<string, object>)Invoke("ReleaseAsset", release);
            return selected == null ? "" : Convert.ToString(selected["name"]);
        }

        private static void Install(string asset, string version)
        {
            string expectedVersion = version;
            if (Has("TryParseReleaseVersion"))
            {
                object[] parsed = new object[] { version, null };
                if (!(bool)Invoke("TryParseReleaseVersion", parsed))
                {
                    throw new InvalidOperationException("The release version is invalid.");
                }
                expectedVersion = (string)parsed[1];
            }
            if (AppVersion.CompareVersions(expectedVersion, AppVersion.Current) <= 0)
            {
                throw new InvalidOperationException("The release version is not newer than " + AppVersion.Current + ".");
            }

            object[] context = new object[] { null, null, null };
            if (!(bool)Invoke("TryGetInstallContext", context))
            {
                throw new InvalidOperationException("Self-update is not available for this copy.");
            }
            string target = (string)context[0];
            string appDir = (string)context[1];
            string powerShell = (string)context[2];

            Invoke("VerifyInstallDirectory", appDir, target);
            string originalHash = Sha256(target);
            long size = new FileInfo(asset).Length;
            string sha256 = Sha256(asset);

            string updateId = Guid.NewGuid().ToString("N");
            string tempDir = Path.Combine(Path.GetTempPath(), "bluetooth_battery_monitor_update_" + updateId);
            string staged = Path.Combine(appDir, "." + AppVersion.ReleaseAssetName + ".update-" + updateId + ".tmp");
            string backup = staged + ".bak";
            string script = Path.Combine(tempDir, "update.ps1");
            int processId = Process.GetCurrentProcess().Id;
            Directory.CreateDirectory(tempDir);

            Invoke("VerifyAssetFile", asset, size, sha256);
            Invoke("VerifyPortableExecutable", asset);
            File.Copy(asset, staged, false);
            Invoke("VerifyAssetFile", staged, size, sha256);
            Invoke("VerifyPortableExecutable", staged);
            if (Has("VerifyManagedRelease"))
            {
                Invoke("VerifyManagedRelease", staged, expectedVersion);
            }
            if (Has("WriteUpdaterScriptCore"))
            {
                Invoke("WriteUpdaterScriptCore", script, staged, target, backup, processId, originalHash);
            }
            else
            {
                Invoke("WriteUpdaterScript", script, staged, target, backup, processId);
            }
            Invoke("StartUpdaterScript", powerShell, script, target);
        }

        private static string Sha256(string path)
        {
            using (SHA256 sha256 = SHA256.Create())
            using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                return BitConverter.ToString(sha256.ComputeHash(stream)).Replace("-", "").ToLowerInvariant();
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
