//! Windows autostart support via the `Run` registry key.
//!
//! Writes/removes a value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` so the application
//! starts automatically when the user logs in. No installer required — the
//! registry entry is managed entirely from the Settings dialog.

/// Registry value name used for this application.
#[cfg(windows)]
const RUN_VALUE_NAME: &str = "JenkinsTray";

/// Full path to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Whether autostart is currently enabled (the registry value exists).
///
/// Returns `false` on non-Windows platforms or when the registry cannot be
/// accessed.
#[cfg(windows)]
pub fn is_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .map(|key| key.get_value::<String, _>(RUN_VALUE_NAME).is_ok())
        .unwrap_or(false)
}

/// Enable autostart: write the current executable path into the `Run` key.
#[cfg(windows)]
pub fn enable() -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let exe_path = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Could not determine executable path: {e}"))?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY)?;
    key.set_value(RUN_VALUE_NAME, &exe_path.to_string_lossy())?;
    Ok(())
}

/// Disable autostart: remove the registry value (if present).
#[cfg(windows)]
pub fn disable() -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_mut(RUN_KEY) {
        // Ignore the error if the value doesn't exist (already disabled).
        let _ = key.delete_value(RUN_VALUE_NAME);
    }
    Ok(())
}

/// Sync the registry state with the desired setting.
///
/// Called whenever the `start_with_windows` general setting is saved, and
/// once at startup so the registry stays consistent with the config file
/// (e.g. after the user moved the exe).
#[cfg(windows)]
pub fn sync(enabled: bool) {
    let result = if enabled { enable() } else { disable() };
    if let Err(e) = result {
        log::error!("Could not update autostart registry entry: {e}");
    }
}

// Non-Windows stubs so call sites don't need #[cfg] everywhere.
#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn sync(_enabled: bool) {
    // No-op on non-Windows platforms.
}
