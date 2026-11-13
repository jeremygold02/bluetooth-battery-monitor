use std::io;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" xmlns:asmv3="urn:schemas-microsoft-com:asm.v3" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <asmv3:trustInfo>
    <asmv3:security>
      <asmv3:requestedPrivileges>
        <asmv3:requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </asmv3:requestedPrivileges>
    </asmv3:security>
  </asmv3:trustInfo>
  <asmv3:application>
    <asmv3:windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </asmv3:windowsSettings>
  </asmv3:application>
</assembly>
"#;

fn release_version() -> String {
    std::env::var("BBM_RELEASE_VERSION")
        .ok()
        .map(|version| version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

fn packed_version(version: &str) -> io::Result<u64> {
    let mut parts = version.split('.').map(str::parse::<u16>);
    let mut next = || {
        parts
            .next()
            .and_then(Result::ok)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid version: {version}")))
    };
    let (major, minor, patch) = (next()?, next()?, next()?);
    if parts.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid version: {version}")));
    }
    Ok((u64::from(major) << 48) | (u64::from(minor) << 32) | (u64::from(patch) << 16))
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-env-changed=BBM_RELEASE_VERSION");
    println!("cargo:rerun-if-changed=icon.ico");
    let version = release_version();
    let packed = packed_version(&version)?;
    println!("cargo:rustc-env=BBM_VERSION={version}");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("icon.ico");
    resource.set_manifest(MANIFEST);
    resource.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
    resource.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
    resource.set("FileVersion", &version);
    resource.set("ProductVersion", &version);
    resource.set("ProductName", "Bluetooth Battery Monitor");
    resource.set("FileDescription", "Bluetooth Battery Monitor");
    resource.set("InternalName", "BluetoothBatteryMonitor");
    resource.set("OriginalFilename", "BluetoothBatteryMonitor.exe");
    resource.compile()
}
