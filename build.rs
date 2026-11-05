// build.rs
// Windows application manifest & resource compilation

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/app.ico"); // Path relative to Cargo.toml
        res.compile().unwrap();
    }
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("ProductName", "OneOCR Searchable PDF Tool");
        res.set("FileDescription", "High-performance Searchable Arabic PDF Generator");
        res.set("LegalCopyright", "Copyright (C) 2026");
        
        // Embed Common Controls 6.0 manifest for modern Windows styling
        res.set_manifest(r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>
"#);

        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile Windows resources: {}", e);
        }
    }
}
