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

    // Compile with the "fluent-light" style: the default "fluent" style follows
    // the OS color scheme at runtime (SlintInternal.color-scheme), so on a dark
    // Windows theme the std-widgets and all default text colors flip to light-on-dark
    // while this port's custom content keeps the original WinForms app's hard-coded
    // light colors (Ui.* backgrounds, row/dialog surfaces) - producing unreadable
    // white text on light backgrounds. The original app was always light-themed, so
    // pin the light variant: with a "-light" style suffix the compiler resolves
    // SlintInternal.color-scheme to a constant `light`, ignoring the OS theme.
    let style_config = slint_build::CompilerConfiguration::default().with_style("fluent-light".into());
    slint_build::compile_with_config("ui/app-window.slint", style_config)
        .expect("Slint build failed");
}
