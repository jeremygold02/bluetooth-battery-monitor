using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Net;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Web.Script.Serialization;
using System.Windows.Forms;

namespace CodexUsageTray
{
    internal sealed class UpdateInfo
    {
        public string CurrentVersion { get; set; }
        public string LatestVersion { get; set; }
        public bool UpdateAvailable { get; set; }
        public bool CanInstall { get; set; }
        public string ReleaseUrl { get; set; }
        public string RepoUrl { get; set; }
        public string AssetName { get; set; }
        public string AssetUrl { get; set; }
        public long AssetSize { get; set; }
        public string AssetSha256 { get; set; }
        public string Message { get; set; }
    }

    internal sealed class UpdateException : Exception
    {
        public UpdateException(string message)
            : base(message)
        {
        }

        public UpdateException(string message, Exception innerException)
            : base(message, innerException)
        {
        }
    }

    internal static class UpdateService
    {
        private const int GitHubTimeoutMilliseconds = 30000;
        private const int DownloadTimeoutMilliseconds = 60000;
        private const int StartupVerificationMilliseconds = 5000;
        private const long MaxReleaseAssetBytes = 100L * 1024L * 1024L;

        public static bool CanInstallUpdates()
        {
            string targetExe;
            string appDir;
            string powerShellPath;
            return TryGetInstallContext(out targetExe, out appDir, out powerShellPath);
        }

        public static UpdateInfo CheckForUpdate()
        {
            Dictionary<string, object> release = LatestRelease();
            if (release == null)
            {
                return new UpdateInfo
                {
                    CurrentVersion = AppVersion.Current,
                    LatestVersion = "",
                    UpdateAvailable = false,
                    CanInstall = false,
                    ReleaseUrl = AppVersion.GitHubRepoUrl,
                    RepoUrl = AppVersion.GitHubRepoUrl,
                    AssetName = "",
                    AssetUrl = "",
                    AssetSize = 0,
                    AssetSha256 = "",
                    Message = "No published releases were found."
                };
            }

            string latestVersion;
            if (!TryParseReleaseVersion(GetString(release, "tag_name"), out latestVersion))
            {
                throw new UpdateException("The latest GitHub release has an invalid version tag. Open the release page to review it manually.");
            }
            string releaseUrl = GetString(release, "html_url");
            if (string.IsNullOrWhiteSpace(releaseUrl))
            {
                releaseUrl = AppVersion.GitHubRepoUrl;
            }

            Dictionary<string, object> asset = ReleaseAsset(release);
            string assetName = asset != null ? GetString(asset, "name") : "";
            string assetUrl = asset != null ? GetString(asset, "browser_download_url") : "";
            long assetSize = asset != null ? GetInt64(asset, "size") : 0;
            string assetSha256 = "";
            bool hasValidDigest = asset != null && TryParseSha256Digest(GetString(asset, "digest"), out assetSha256);
            if (!hasValidDigest)
            {
                assetSha256 = "";
            }

            bool updateAvailable = AppVersion.CompareVersions(latestVersion, AppVersion.Current) > 0;
            bool hasInstallAsset = !string.IsNullOrWhiteSpace(assetUrl);
            Uri parsedAssetUri;
            bool hasValidAssetUri = hasInstallAsset && TryGetReleaseAssetUri(assetUrl, out parsedAssetUri);
            bool hasValidSize = assetSize > 0 && assetSize <= MaxReleaseAssetBytes;
            bool hasIntegrityMetadata = hasValidSize && hasValidDigest;
            bool installSupported = CanInstallUpdates();
            bool canInstall = updateAvailable && hasValidAssetUri && hasIntegrityMetadata && installSupported;

            string message;
            if (updateAvailable && !hasInstallAsset)
            {
                message = "Update available: " + latestVersion + ", but no Windows exe asset was attached.";
            }
            else if (updateAvailable && !hasValidAssetUri)
            {
                message = "Update available: " + latestVersion + ", but its download address is not valid for automatic installation.";
            }
            else if (updateAvailable && !hasIntegrityMetadata)
            {
                message = "Update available: " + latestVersion + ", but automatic installation is unavailable because the release has no valid size and SHA-256 metadata.";
            }
            else if (updateAvailable && !installSupported)
            {
                message = "Update available: " + latestVersion + ". Automatic installation is not available in this copy.";
            }
            else if (updateAvailable)
            {
                message = "Update available: " + latestVersion + ".";
            }
            else
            {
                message = "You are on the latest published version (" + latestVersion + ").";
            }

            return new UpdateInfo
            {
                CurrentVersion = AppVersion.Current,
                LatestVersion = latestVersion,
                UpdateAvailable = updateAvailable,
                CanInstall = canInstall,
                ReleaseUrl = releaseUrl,
                RepoUrl = AppVersion.GitHubRepoUrl,
                AssetName = assetName,
                AssetUrl = assetUrl,
                AssetSize = assetSize,
                AssetSha256 = assetSha256,
                Message = message
            };
        }

        public static string InstallUpdate(UpdateInfo update)
        {
            if (update == null || !update.UpdateAvailable)
            {
                throw new UpdateException("No update is available.");
            }
            string expectedVersion;
            if (!TryParseReleaseVersion(update.LatestVersion, out expectedVersion) ||
                AppVersion.CompareVersions(expectedVersion, AppVersion.Current) <= 0)
            {
                throw new UpdateException("The update version is invalid or is not newer than this app.");
            }
            if (string.IsNullOrWhiteSpace(update.AssetUrl))
            {
                throw new UpdateException("The latest release does not include a Windows exe asset.");
            }
            if (!string.Equals(ComparableExeStem(update.AssetName), ComparableExeStem(AppVersion.ReleaseAssetName), StringComparison.OrdinalIgnoreCase))
            {
                throw new UpdateException("The release asset name is not valid for automatic installation.");
            }
            if (update.AssetSize <= 0 || update.AssetSize > MaxReleaseAssetBytes || !IsSha256Hex(update.AssetSha256))
            {
                throw new UpdateException("The release does not include valid integrity metadata. Open the release page to install it manually.");
            }

            Uri assetUri;
            if (!TryGetReleaseAssetUri(update.AssetUrl, out assetUri))
            {
                throw new UpdateException("The release download address is not valid for automatic installation.");
            }

            string targetExe;
            string appDir;
            string powerShellPath;
            if (!TryGetInstallContext(out targetExe, out appDir, out powerShellPath))
            {
                throw new UpdateException("Self-update is only available in the packaged Windows app with Windows PowerShell installed.");
            }

            VerifyInstallDirectory(appDir, targetExe);

            string tempDir = "";
            string downloadedExe = "";
            string stagedExe = "";
            string scriptPath = "";
            bool updaterStarted = false;

            try
            {
                string originalTargetHash = ComputeSha256(targetExe);
                string updateId = Guid.NewGuid().ToString("N");
                tempDir = Path.Combine(Path.GetTempPath(), "bluetooth_battery_monitor_update_" + updateId);
                downloadedExe = Path.Combine(tempDir, AppVersion.ReleaseAssetName + ".download");
                stagedExe = Path.Combine(appDir, "." + AppVersion.ReleaseAssetName + ".update-" + updateId + ".tmp");
                string backupExe = stagedExe + ".bak";
                scriptPath = Path.Combine(tempDir, "update.ps1");

                Directory.CreateDirectory(tempDir);
                DownloadFile(assetUri, downloadedExe, update.AssetSize);
                VerifyAssetFile(downloadedExe, update.AssetSize, update.AssetSha256);
                VerifyPortableExecutable(downloadedExe);

                File.Copy(downloadedExe, stagedExe, false);
                VerifyAssetFile(stagedExe, update.AssetSize, update.AssetSha256);
                VerifyPortableExecutable(stagedExe);
                VerifyManagedRelease(stagedExe, expectedVersion);

                WriteUpdaterScriptCore(scriptPath, stagedExe, targetExe, backupExe, Process.GetCurrentProcess().Id, originalTargetHash);
                TryDeleteFile(downloadedExe);
                StartUpdaterScript(powerShellPath, scriptPath, targetExe);
                updaterStarted = true;

                return "Update verified. Bluetooth Battery Monitor will restart automatically.";
            }
            catch (UpdateException)
            {
                throw;
            }
            catch (WebException ex)
            {
                if (ex.Status == WebExceptionStatus.Timeout)
                {
                    throw new UpdateException("The update download timed out. Try again in a moment.", ex);
                }

                throw new UpdateException("The update could not be downloaded securely. Check your connection and try again.", ex);
            }
            catch (UnauthorizedAccessException ex)
            {
                throw new UpdateException("The app folder does not allow automatic updates. Install the release manually instead.", ex);
            }
            catch (IOException ex)
            {
                throw new UpdateException("The update could not be prepared safely. No app files were replaced.", ex);
            }
            catch (Exception ex)
            {
                throw new UpdateException("The update installer could not be started. No app files were replaced.", ex);
            }
            finally
            {
                if (!updaterStarted)
                {
                    TryDeleteFile(stagedExe);
                    TryDeleteDirectory(tempDir);
                }
            }
        }

        internal static bool TryParseReleaseVersion(string tag, out string version)
        {
            version = "";
            if (string.IsNullOrEmpty(tag)) return false;
            string value = tag.StartsWith("v", StringComparison.Ordinal) ? tag.Substring(1) : tag;
            string[] parts = value.Split('.');
            if (parts.Length != 3) return false;
            foreach (string part in parts)
            {
                int number;
                if (part.Length == 0 || (part.Length > 1 && part[0] == '0') ||
                    !int.TryParse(part, NumberStyles.None, CultureInfo.InvariantCulture, out number)) return false;
            }
            version = value;
            return true;
        }

        internal static void VerifyManagedRelease(string path, string expectedVersion)
        {
            try
            {
                string version;
                if (!TryParseReleaseVersion(expectedVersion, out version))
                    throw new UpdateException("The release version is invalid.");

                // Reflection-only loading inspects metadata without executing code in the downloaded file.
                // Loading bytes avoids keeping the staged executable locked after validation.
                Assembly assembly = Assembly.ReflectionOnlyLoad(File.ReadAllBytes(path));
                Type buildVersion = assembly.GetType("CodexUsageTray.BuildVersion", false, false);
                if (!string.Equals(assembly.GetName().Name, "BluetoothBatteryMonitor", StringComparison.Ordinal) ||
                    buildVersion == null ||
                    !string.Equals(ReadMetadataConstant(buildVersion, "Version"), version, StringComparison.Ordinal) ||
                    !string.Equals(ReadMetadataConstant(buildVersion, "GitHubOwner"), AppVersion.GitHubOwner, StringComparison.OrdinalIgnoreCase) ||
                    !string.Equals(ReadMetadataConstant(buildVersion, "GitHubRepo"), AppVersion.GitHubRepo, StringComparison.OrdinalIgnoreCase))
                    throw new UpdateException("The downloaded app identity or version does not match the GitHub release.");
            }
            catch (UpdateException) { throw; }
            catch (Exception ex)
            {
                throw new UpdateException("The downloaded app metadata could not be verified. No app files were replaced.", ex);
            }
        }

        private static string ReadMetadataConstant(Type type, string name)
        {
            FieldInfo field = type.GetField(name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static);
            return field != null && field.IsLiteral && !field.IsInitOnly ? field.GetRawConstantValue() as string : null;
        }

        private static Dictionary<string, object> LatestRelease()
        {
            try
            {
                return GithubJson("/releases/latest");
            }
            catch (WebException ex)
            {
                HttpWebResponse response = ex.Response as HttpWebResponse;
                if (response != null && response.StatusCode == HttpStatusCode.NotFound)
                {
                    response.Close();
                    return null;
                }
                if (response != null)
                {
                    response.Close();
                }
                if (ex.Status == WebExceptionStatus.Timeout)
                {
                    throw new UpdateException("GitHub took too long to respond. Try again in a moment.", ex);
                }

                throw new UpdateException("Could not check GitHub for updates. Check your connection and try again.", ex);
            }
            catch (Exception ex)
            {
                throw new UpdateException("GitHub returned update information that could not be read safely. Try again later.", ex);
            }
        }

        private static Dictionary<string, object> GithubJson(string path)
        {
            HttpWebRequest request = (HttpWebRequest)WebRequest.Create(AppVersion.GitHubApiRepo + path);
            request.Accept = "application/vnd.github+json";
            request.UserAgent = "Bluetooth-Battery-Monitor";
            request.Headers["X-GitHub-Api-Version"] = "2022-11-28";
            request.Timeout = GitHubTimeoutMilliseconds;

            using (HttpWebResponse response = (HttpWebResponse)request.GetResponse())
            using (Stream stream = response.GetResponseStream())
            using (StreamReader reader = new StreamReader(stream, Encoding.UTF8))
            {
                object parsed = new JavaScriptSerializer().DeserializeObject(reader.ReadToEnd());
                return parsed as Dictionary<string, object>;
            }
        }

        private static Dictionary<string, object> ReleaseAsset(Dictionary<string, object> release)
        {
            object assetsObject;
            if (!release.TryGetValue("assets", out assetsObject) || assetsObject == null)
            {
                return null;
            }

            IEnumerable assets = assetsObject as IEnumerable;
            if (assets == null)
            {
                return null;
            }

            string expectedStem = ComparableExeStem(AppVersion.ReleaseAssetName);
            foreach (object item in assets)
            {
                Dictionary<string, object> asset = item as Dictionary<string, object>;
                if (asset == null)
                {
                    continue;
                }

                if (string.Equals(ComparableExeStem(GetString(asset, "name")), expectedStem, StringComparison.OrdinalIgnoreCase))
                {
                    return asset;
                }
            }

            return null;
        }

        private static string ComparableExeStem(string name)
        {
            if (string.IsNullOrWhiteSpace(name) || !string.Equals(Path.GetExtension(name), ".exe", StringComparison.OrdinalIgnoreCase))
            {
                return "";
            }

            string stem = Path.GetFileNameWithoutExtension(name).ToLowerInvariant();
            StringBuilder builder = new StringBuilder();
            for (int i = 0; i < stem.Length; i++)
            {
                if (char.IsLetterOrDigit(stem[i]))
                {
                    builder.Append(stem[i]);
                }
            }

            return builder.ToString();
        }

        private static string GetString(Dictionary<string, object> values, string key)
        {
            object value;
            if (values != null && values.TryGetValue(key, out value) && value != null)
            {
                return Convert.ToString(value);
            }

            return "";
        }

        private static long GetInt64(Dictionary<string, object> values, string key)
        {
            object value;
            if (values == null || !values.TryGetValue(key, out value) || value == null || value is bool)
            {
                return 0;
            }

            try
            {
                decimal numericValue = Convert.ToDecimal(value, CultureInfo.InvariantCulture);
                if (numericValue < 0 || numericValue > long.MaxValue || decimal.Truncate(numericValue) != numericValue)
                {
                    return 0;
                }

                return (long)numericValue;
            }
            catch (FormatException)
            {
                return 0;
            }
            catch (InvalidCastException)
            {
                return 0;
            }
            catch (OverflowException)
            {
                return 0;
            }
        }

        private static bool TryParseSha256Digest(string digest, out string sha256)
        {
            sha256 = "";
            if (string.IsNullOrWhiteSpace(digest))
            {
                return false;
            }

            const string prefix = "sha256:";
            string value = digest.Trim();
            if (!value.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            value = value.Substring(prefix.Length);
            if (!IsSha256Hex(value))
            {
                return false;
            }

            sha256 = value.ToLowerInvariant();
            return true;
        }

        private static bool IsSha256Hex(string value)
        {
            if (string.IsNullOrEmpty(value) || value.Length != 64)
            {
                return false;
            }

            for (int i = 0; i < value.Length; i++)
            {
                char character = value[i];
                bool isHex = (character >= '0' && character <= '9')
                    || (character >= 'a' && character <= 'f')
                    || (character >= 'A' && character <= 'F');
                if (!isHex)
                {
                    return false;
                }
            }

            return true;
        }

        private static bool TryGetReleaseAssetUri(string value, out Uri assetUri)
        {
            assetUri = null;
            Uri parsed;
            if (!Uri.TryCreate(value, UriKind.Absolute, out parsed)
                || !string.Equals(parsed.Scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase)
                || !string.Equals(parsed.Host, "github.com", StringComparison.OrdinalIgnoreCase)
                || !parsed.IsDefaultPort
                || !string.IsNullOrEmpty(parsed.UserInfo)
                || !string.IsNullOrEmpty(parsed.Fragment))
            {
                return false;
            }

            string expectedPrefix = "/" + AppVersion.GitHubOwner + "/" + AppVersion.GitHubRepo + "/releases/download/";
            if (!parsed.AbsolutePath.StartsWith(expectedPrefix, StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            assetUri = parsed;
            return true;
        }

        private static bool TryGetInstallContext(out string targetExe, out string appDir, out string powerShellPath)
        {
            targetExe = "";
            appDir = "";
            powerShellPath = "";

            if (Environment.OSVersion.Platform != PlatformID.Win32NT)
            {
                return false;
            }

            try
            {
                string executablePath = Application.ExecutablePath;
                if (string.IsNullOrWhiteSpace(executablePath))
                {
                    return false;
                }

                targetExe = Path.GetFullPath(executablePath);
                appDir = Path.GetDirectoryName(targetExe);
                if (!File.Exists(targetExe)
                    || string.IsNullOrWhiteSpace(appDir)
                    || !Directory.Exists(appDir)
                    || !string.Equals(Path.GetExtension(targetExe), ".exe", StringComparison.OrdinalIgnoreCase)
                    || (File.GetAttributes(targetExe) & FileAttributes.ReadOnly) != 0)
                {
                    return false;
                }

                string systemDirectory = Environment.GetFolderPath(Environment.SpecialFolder.System);
                if (!string.IsNullOrWhiteSpace(systemDirectory))
                {
                    powerShellPath = Path.Combine(systemDirectory, @"WindowsPowerShell\v1.0\powershell.exe");
                }

                if (!File.Exists(powerShellPath))
                {
                    string windowsDirectory = Environment.GetEnvironmentVariable("WINDIR");
                    powerShellPath = string.IsNullOrWhiteSpace(windowsDirectory)
                        ? ""
                        : Path.Combine(windowsDirectory, @"System32\WindowsPowerShell\v1.0\powershell.exe");
                }

                return File.Exists(powerShellPath);
            }
            catch (Exception)
            {
                targetExe = "";
                appDir = "";
                powerShellPath = "";
                return false;
            }
        }

        private static void VerifyInstallDirectory(string appDir, string targetExe)
        {
            string probeId = Guid.NewGuid().ToString("N");
            string probePrefix = Path.Combine(appDir, "." + Path.GetFileName(targetExe) + ".update-write-" + probeId);
            string probeTarget = probePrefix + ".target";
            string probeReplacement = probePrefix + ".replacement";
            string probeBackup = probePrefix + ".backup";

            try
            {
                using (FileStream probe = new FileStream(probeTarget, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                {
                    probe.WriteByte(0);
                    probe.Flush();
                }
                using (FileStream probe = new FileStream(probeReplacement, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                {
                    probe.WriteByte(1);
                    probe.Flush();
                }

                File.Replace(probeReplacement, probeTarget, probeBackup, true);
                if (!File.Exists(probeTarget) || !File.Exists(probeBackup))
                {
                    throw new IOException("Atomic replacement preflight did not produce the expected files.");
                }
            }
            catch (Exception ex)
            {
                throw new UpdateException("The app folder does not support safe automatic replacement. Install the release manually instead.", ex);
            }
            finally
            {
                TryDeleteFile(probeTarget);
                TryDeleteFile(probeReplacement);
                TryDeleteFile(probeBackup);
            }

            if (File.Exists(probeTarget) || File.Exists(probeReplacement) || File.Exists(probeBackup))
            {
                throw new UpdateException("The app folder could not clean up an update preflight file. Automatic installation was stopped.");
            }
        }

        private static void DownloadFile(Uri url, string target, long expectedSize)
        {
            HttpWebRequest request = (HttpWebRequest)WebRequest.Create(url);
            request.Accept = "application/octet-stream";
            request.UserAgent = "Bluetooth-Battery-Monitor";
            request.AllowAutoRedirect = true;
            request.Timeout = DownloadTimeoutMilliseconds;
            request.ReadWriteTimeout = DownloadTimeoutMilliseconds;

            using (HttpWebResponse response = (HttpWebResponse)request.GetResponse())
            {
                if (response.ResponseUri == null
                    || !string.Equals(response.ResponseUri.Scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase))
                {
                    throw new UpdateException("The update download was redirected to an insecure address. No files were replaced.");
                }
                if (response.ContentLength >= 0 && response.ContentLength != expectedSize)
                {
                    throw new UpdateException("The update download size did not match the release metadata. No files were replaced.");
                }

                using (Stream source = response.GetResponseStream())
                {
                    if (source == null)
                    {
                        throw new UpdateException("The update download did not contain any data. No files were replaced.");
                    }

                    using (FileStream destination = new FileStream(target, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                    {
                        byte[] buffer = new byte[81920];
                        long totalBytes = 0;
                        int bytesRead;
                        while ((bytesRead = source.Read(buffer, 0, buffer.Length)) > 0)
                        {
                            if (bytesRead > expectedSize - totalBytes)
                            {
                                throw new UpdateException("The update download exceeded the expected size. No files were replaced.");
                            }

                            destination.Write(buffer, 0, bytesRead);
                            totalBytes += bytesRead;
                        }

                        destination.Flush();
                        if (totalBytes != expectedSize)
                        {
                            throw new UpdateException("The update download was incomplete. No files were replaced.");
                        }
                    }
                }
            }
        }

        private static void VerifyAssetFile(string path, long expectedSize, string expectedSha256)
        {
            FileInfo file = new FileInfo(path);
            if (!file.Exists || file.Length != expectedSize)
            {
                throw new UpdateException("The update file size failed verification. No files were replaced.");
            }

            string actualSha256 = ComputeSha256(path);
            if (!string.Equals(actualSha256, expectedSha256, StringComparison.OrdinalIgnoreCase))
            {
                throw new UpdateException("The update failed SHA-256 verification. No files were replaced.");
            }
        }

        private static string ComputeSha256(string path)
        {
            using (SHA256 sha256 = SHA256.Create())
            using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read))
                return BitConverter.ToString(sha256.ComputeHash(stream)).Replace("-", "").ToLowerInvariant();
        }

        private static void VerifyPortableExecutable(string path)
        {
            using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                if (stream.ReadByte() != 'M' || stream.ReadByte() != 'Z')
                {
                    throw new UpdateException("The verified release asset is not a Windows executable. No files were replaced.");
                }
            }
        }

        private static void TryDeleteFile(string path)
        {
            try
            {
                if (!string.IsNullOrWhiteSpace(path) && File.Exists(path))
                {
                    File.Delete(path);
                }
            }
            catch
            {
            }
        }

        private static void TryDeleteDirectory(string path)
        {
            try
            {
                if (!string.IsNullOrWhiteSpace(path) && Directory.Exists(path))
                {
                    Directory.Delete(path, true);
                }
            }
            catch
            {
            }
        }

        private static void WriteUpdaterScript(string scriptPath, string stagedExe, string targetExe, string backupExe, int pid)
        {
            WriteUpdaterScriptCore(scriptPath, stagedExe, targetExe, backupExe, pid,
                File.Exists(targetExe) ? ComputeSha256(targetExe) : "");
        }

        private static void WriteUpdaterScriptCore(string scriptPath, string stagedExe, string targetExe, string backupExe, int pid, string targetHash)
        {
            string logPath = Path.ChangeExtension(scriptPath, ".log");
            string tempDir = Path.GetDirectoryName(scriptPath);
            string rollbackStage = stagedExe + ".rollback";
            string failedExe = stagedExe + ".failed";
            string lockName;
            using (SHA256 sha256 = SHA256.Create())
            {
                byte[] pathBytes = Encoding.UTF8.GetBytes(Path.GetFullPath(targetExe).ToUpperInvariant());
                lockName = @"Global\BluetoothBatteryMonitor.Update." + BitConverter.ToString(sha256.ComputeHash(pathBytes)).Replace("-", "");
            }
            string[] lines = new string[]
            {
                "$ErrorActionPreference = 'Stop'",
                "$target = " + PowerShellLiteral(targetExe),
                "$stagedExe = " + PowerShellLiteral(stagedExe),
                "$backup = " + PowerShellLiteral(backupExe),
                "$rollbackStage = " + PowerShellLiteral(rollbackStage),
                "$failedExe = " + PowerShellLiteral(failedExe),
                "$appDir = " + PowerShellLiteral(Path.GetDirectoryName(targetExe)),
                "$tempDir = " + PowerShellLiteral(tempDir),
                "$pidToWait = " + pid,
                "$log = " + PowerShellLiteral(logPath),
                "$startupCheckMilliseconds = " + StartupVerificationMilliseconds,
                "$expectedTargetHash = " + PowerShellLiteral(targetHash),
                "$mutexName = " + PowerShellLiteral(lockName),
                "",
                "function Write-UpdateLog {",
                "  param([string]$Message)",
                "  try {",
                "    Add-Content -LiteralPath $log -Value \"$(Get-Date -Format o) $Message\" -ErrorAction Stop",
                "  } catch {",
                "  }",
                "}",
                "",
                "function Get-AppProcesses {",
                "  @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {",
                "    $_.ExecutablePath -and [string]::Equals($_.ExecutablePath, $target, [System.StringComparison]::OrdinalIgnoreCase)",
                "  })",
                "}",
                "",
                "function Restore-Backup {",
                "  if (-not (Test-Path -LiteralPath $backup -PathType Leaf)) {",
                "    throw 'The previous executable backup is missing.'",
                "  }",
                "",
                "  $newProcesses = @(Get-AppProcesses)",
                "  foreach ($process in $newProcesses) {",
                "    Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue",
                "  }",
                "  if ($newProcesses.Count -gt 0) {",
                "    Start-Sleep -Milliseconds 800",
                "  }",
                "",
                "  Remove-Item -LiteralPath $rollbackStage -Force -ErrorAction SilentlyContinue",
                "  Remove-Item -LiteralPath $failedExe -Force -ErrorAction SilentlyContinue",
                "  Copy-Item -LiteralPath $backup -Destination $rollbackStage -ErrorAction Stop",
                "",
                "  $restored = $false",
                "  for ($attempt = 1; $attempt -le 20; $attempt++) {",
                "    try {",
                "      [System.IO.File]::Replace($rollbackStage, $target, $failedExe, $true)",
                "      $restored = $true",
                "      break",
                "    } catch {",
                "      Write-UpdateLog \"Rollback attempt $attempt failed: $($_.Exception.Message)\"",
                "      Start-Sleep -Milliseconds 500",
                "    }",
                "  }",
                "  if (-not $restored) {",
                "    throw 'Could not restore the previous executable.'",
                "  }",
                "",
                "  Remove-Item -LiteralPath $failedExe -Force -ErrorAction SilentlyContinue",
                "  $restoredProcess = Start-Process -FilePath $target -ArgumentList '--skip-startup-update', '--update-failed' -WorkingDirectory $appDir -WindowStyle Hidden -PassThru",
                "  if ($null -eq $restoredProcess -or $restoredProcess.WaitForExit($startupCheckMilliseconds)) {",
                "    throw 'The restored app did not remain running.'",
                "  }",
                "}",
                "",
                "$replacementInstalled = $false",
                "$succeeded = $false",
                "$exitCode = 0",
                "$updateMutex = $null",
                "$lockAcquired = $false",
                "$skipRestart = $false",
                "Write-UpdateLog 'Starting Bluetooth Battery Monitor update.'",
                "try {",
                "  if (-not (Test-Path -LiteralPath $stagedExe -PathType Leaf)) {",
                "    throw 'The staged update file is missing.'",
                "  }",
                "  if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {",
                "    throw 'The installed executable is missing.'",
                "  }",
                "  $skipRestart = $true",
                "  $updateMutex = [System.Threading.Mutex]::new($false, $mutexName)",
                "  try { $lockAcquired = $updateMutex.WaitOne(120000) } catch [System.Threading.AbandonedMutexException] { $lockAcquired = $true }",
                "  if (-not $lockAcquired) {",
                "    $skipRestart = $true",
                "    throw 'Another update is still in progress.'",
                "  }",
                "  $skipRestart = $false",
                "  if (-not $expectedTargetHash -or (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $expectedTargetHash) {",
                "    $skipRestart = $true",
                "    throw 'The installed executable changed after this update was prepared.'",
                "  }",
                "",
                "  $deadline = (Get-Date).AddSeconds(20)",
                "  while ((Get-Process -Id $pidToWait -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) {",
                "    Start-Sleep -Milliseconds 500",
                "  }",
                "",
                "  $remaining = @(Get-AppProcesses)",
                "  if ($remaining.Count -gt 0) {",
                "    Write-UpdateLog \"Stopping $($remaining.Count) old app process(es).\"",
                "    foreach ($process in $remaining) {",
                "      Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue",
                "    }",
                "    Start-Sleep -Milliseconds 800",
                "  }",
                "  if (@(Get-AppProcesses).Count -gt 0) {",
                "    throw 'The installed app is still running.'",
                "  }",
                "  if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $expectedTargetHash) {",
                "    $skipRestart = $true",
                "    throw 'The installed executable changed before replacement.'",
                "  }",
                "",
                "  if (Test-Path -LiteralPath $backup) {",
                "    throw 'The update backup path already exists.'",
                "  }",
                "",
                "  $replaced = $false",
                "  for ($attempt = 1; $attempt -le 30; $attempt++) {",
                "    try {",
                "      [System.IO.File]::Replace($stagedExe, $target, $backup, $true)",
                "      $replaced = $true",
                "      break",
                "    } catch {",
                "      Write-UpdateLog \"Replace attempt $attempt failed: $($_.Exception.Message)\"",
                "      Start-Sleep -Seconds 1",
                "    }",
                "  }",
                "  if (-not $replaced) {",
                "    throw 'Could not replace the app after 30 attempts.'",
                "  }",
                "  $replacementInstalled = $true",
                "",
                "  Write-UpdateLog 'Starting updated app.'",
                "  $startedProcess = Start-Process -FilePath $target -ArgumentList '--skip-startup-update', '--update-complete' -WorkingDirectory $appDir -WindowStyle Hidden -PassThru",
                "  if ($null -eq $startedProcess) {",
                "    throw 'The updated app process could not be started.'",
                "  }",
                "  if ($startedProcess.WaitForExit($startupCheckMilliseconds)) {",
                "    throw 'The updated app exited during startup.'",
                "  }",
                "",
                "  Write-UpdateLog 'Updated app remained running; update completed.'",
                "  $succeeded = $true",
                "} catch {",
                "  $failureMessage = $_.Exception.Message",
                "  Write-UpdateLog \"Update failed: $failureMessage\"",
                "  try {",
                "    if ($replacementInstalled) {",
                "      Write-UpdateLog 'Restoring previous executable.'",
                "      Restore-Backup",
                "      Write-UpdateLog 'Previous executable restored and restarted.'",
                "    } elseif (-not $skipRestart) {",
                "      $restartDeadline = (Get-Date).AddSeconds(5)",
                "      while ((Get-Process -Id $pidToWait -ErrorAction SilentlyContinue) -and (Get-Date) -lt $restartDeadline) {",
                "        Start-Sleep -Milliseconds 250",
                "      }",
                "      if (@(Get-AppProcesses).Count -eq 0) {",
                "        Start-Process -FilePath $target -ArgumentList '--skip-startup-update', '--update-failed' -WorkingDirectory $appDir -WindowStyle Hidden | Out-Null",
                "        Write-UpdateLog 'Installed executable restarted after update preparation failed.'",
                "      }",
                "    }",
                "  } catch {",
                "    Write-UpdateLog \"Rollback failed: $($_.Exception.Message)\"",
                "  }",
                "  $exitCode = 1",
                "} finally {",
                "  if ($lockAcquired) { $updateMutex.ReleaseMutex() }",
                "  if ($null -ne $updateMutex) { $updateMutex.Dispose() }",
                "  Remove-Item -LiteralPath $stagedExe -Force -ErrorAction SilentlyContinue",
                "  Remove-Item -LiteralPath $rollbackStage -Force -ErrorAction SilentlyContinue",
                "  Remove-Item -LiteralPath $failedExe -Force -ErrorAction SilentlyContinue",
                "  if ($succeeded) {",
                "    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue",
                "    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue",
                "  }",
                "  Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue",
                "  if ($succeeded) {",
                "    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue",
                "  }",
                "}",
                "exit $exitCode",
                ""
            };

            File.WriteAllText(scriptPath, string.Join(Environment.NewLine, lines), Encoding.UTF8);
        }

        private static void StartUpdaterScript(string powerShellPath, string scriptPath, string targetExe)
        {
            ProcessStartInfo startInfo = new ProcessStartInfo();
            startInfo.FileName = powerShellPath;
            startInfo.Arguments = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File " + QuoteArgument(scriptPath);
            startInfo.WorkingDirectory = Path.GetDirectoryName(targetExe);
            startInfo.UseShellExecute = false;
            startInfo.CreateNoWindow = true;
            startInfo.WindowStyle = ProcessWindowStyle.Hidden;

            using (Process updater = Process.Start(startInfo))
            {
                if (updater == null || updater.WaitForExit(750))
                {
                    throw new UpdateException("The update installer did not start correctly. No app files were replaced.");
                }
            }
        }

        private static string PowerShellLiteral(string value)
        {
            return "'" + (value ?? "").Replace("'", "''") + "'";
        }

        private static string QuoteArgument(string value)
        {
            return "\"" + (value ?? "").Replace("\"", "\\\"") + "\"";
        }

    }
}
