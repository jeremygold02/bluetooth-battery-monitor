using System;
using System.IO;
using System.Reflection;

namespace CodexUsageTray
{
    internal static class EmbeddedAssemblyResolver
    {
        private const string ResourcePrefix = "BluetoothBatteryMonitor.Embedded.";

        internal static void Register()
        {
            AppDomain.CurrentDomain.AssemblyResolve += Resolve;
        }

        private static Assembly Resolve(object sender, ResolveEventArgs args)
        {
            string name = new AssemblyName(args.Name).Name;
            Assembly application = Assembly.GetExecutingAssembly();
            using (Stream stream = application.GetManifestResourceStream(ResourcePrefix + name + ".dll"))
            {
                if (stream == null)
                {
                    return null;
                }

                using (MemoryStream bytes = new MemoryStream())
                {
                    stream.CopyTo(bytes);
                    return Assembly.Load(bytes.ToArray());
                }
            }
        }
    }
}
