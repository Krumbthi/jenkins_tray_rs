fn main() {
    // Generate the versioned window title source (original: MainForm sets
    // `Text = string.Format("{0} v{1}.{2}.{3}", ProductName, FileMajor,
    // FileMinor, FileBuild)` from FileVersionInfo). Slint has no runtime
    // setter for the window title, so the value is baked in at build time.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    let patch = parts.next().unwrap_or("0");
    let content = format!(
        "// Generated from CARGO_PKG_VERSION ({version}) by build.rs - do not edit.\n\
         export global AppVersion {{\n    \
             public pure function window-title() -> string {{\n        \
                 \"Jenkins Tray v{major}.{minor}.{patch}\"\n    \
             }}\n\
         }}\n"
    );

    let path = std::path::Path::new("ui/version.slint");
    let current = std::fs::read_to_string(path).ok();
    if current.as_deref() != Some(content.as_str()) {
        std::fs::write(path, &content).expect("could not write ui/version.slint");
    }

    slint_build::compile("ui/app-window.slint").expect("Slint build failed");
}
