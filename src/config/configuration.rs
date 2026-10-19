/// Config file name (renamed from the original C# `jenkins.configuration`).
pub const CONFIGURATION_FILE: &str = "jenkins_tray.json";

/// Previous configuration file name, used by the original C# application and
/// by earlier builds of this port. Still read once for migration.
pub const LEGACY_CONFIGURATION_FILE: &str = "jenkins.configuration";

/// Directory name for Jenkins Tray app data (mirrors C# JENKINS_TRAY_DIRECTORY)
pub const JENKINS_TRAY_DIRECTORY: &str = "Jenkins Tray";

/// Load configuration from disk.
///
/// If `jenkins_tray.json` does not exist yet, an older
/// `jenkins.configuration` file is migrated (read and re-saved under the new
/// name). Files that cannot be parsed (e.g. one written by the original C#
/// application, whose format differs - see the deviation note on
/// [`crate::model::Configuration`]) are ignored with a warning instead of
/// aborting the startup.
///
/// Passwords are retrieved from the OS credential manager (keyring) if available;
/// otherwise the plaintext value from the config file is kept (migration from
/// older builds). The next save will move any remaining plaintext passwords
/// into the OS credential manager.
pub fn load_configuration() -> anyhow::Result<crate::model::Configuration> {
    let path = config_file_path()?;

    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let mut config: crate::model::Configuration = serde_json::from_str(&data)?;
        migrate_passwords(&mut config);
        return Ok(normalize(config));
    }

    // One-time migration from the previous file name.
    let legacy_path = config_file_path_for(LEGACY_CONFIGURATION_FILE)?;
    if legacy_path.exists() {
        let legacy: Option<crate::model::Configuration> = std::fs::read_to_string(&legacy_path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok());
        match legacy {
            Some(mut config) => {
                log::info!(
                    "Migrating configuration from {} to {}",
                    legacy_path.display(),
                    path.display()
                );
                migrate_passwords(&mut config);
                let config = normalize(config);
                if let Err(e) = save_configuration(&config) {
                    log::warn!("Could not write the migrated configuration: {e}");
                }
                return Ok(config);
            }
            None => {
                log::warn!(
                    "Ignoring unreadable legacy configuration file {}",
                    legacy_path.display()
                );
            }
        }
    }

    Ok(crate::model::Configuration::default())
}

/// Get the configuration file path.
///
/// The file lives in `%APPDATA%\Jenkins Tray` (equivalent of the original
/// `Environment.GetFolderPath(SpecialFolder.ApplicationData)` +
/// `JENKINS_TRAY_DIRECTORY`).
pub fn config_file_path() -> anyhow::Result<std::path::PathBuf> {
    config_file_path_for(CONFIGURATION_FILE)
}

/// Get the path of a configuration file with the given name.
fn config_file_path_for(file_name: &str) -> anyhow::Result<std::path::PathBuf> {
    let app_data_dir = app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine app data directory"))?;

    let path = app_data_dir.join(JENKINS_TRAY_DIRECTORY).join(file_name);
    Ok(path)
}

/// Per-user application data directory, equivalent of
/// `Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData)`:
/// `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` (default `~/.config`) on Unix.
fn app_data_dir() -> Option<std::path::PathBuf> {
    // Windows: %APPDATA% (e.g. C:\Users\<user>\AppData\Roaming).
    #[cfg(windows)]
    {
        if let Some(dir) = env_dir("APPDATA") {
            return Some(dir);
        }
        // Defensive fallback when APPDATA is not exported.
        if let Some(dir) = env_dir("USERPROFILE") {
            return Some(dir.join("AppData").join("Roaming"));
        }
    }

    // Unix: what .NET reports for SpecialFolder.ApplicationData there.
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        return Some(dir);
    }
    env_dir("HOME").map(|home| home.join(".config"))
}

/// Reads an environment variable as a directory path, ignoring empty values.
fn env_dir(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

/// Cross-platform credential migration: move any plaintext passwords from the
/// config file into the OS credential manager (keyring), clearing them from
/// the config file. This is called after every load so that passwords added
/// via the Edit Server dialog (which temporarily stores them in the config)
/// get promoted to the OS credential manager on the next save.
fn migrate_passwords(config: &mut crate::model::Configuration) {
    use keyring::Entry;

    for server in &mut config.servers {
        if let Some(password) = server.password.take()
            && !password.is_empty()
        {
            let service = "jenkins_tray";
            let account = &server.url;
            // Try to store the password in the OS credential manager.
            // If the keyring backend is unavailable or the operation fails,
            // we keep the plaintext in the config file and try again next time.
            if let Ok(entry) = Entry::new(service, account) {
                let _ = entry.set_password(&password);
            }
        }
    }
}

/// Post-load fixes for values an earlier build of this port could write.
fn normalize(mut config: crate::model::Configuration) -> crate::model::Configuration {
    // An earlier build defaulted `refresh_interval_in_seconds` to 0; the
    // original constructor default is 15 (DEFAULT_TIME_BETWEEN_UPDATES) and a
    // zero interval would reschedule the update timer immediately, polling
    // in a busy loop.
    if config.general_settings.refresh_interval_in_seconds == 0 {
        config.general_settings.refresh_interval_in_seconds = 15;
    }
    config
}

/// Save configuration to disk.
///
/// Before writing, any passwords still present in the config are moved to the
/// OS credential manager (keyring) so that the JSON file never contains
/// plaintext passwords longer than one save cycle.
pub fn save_configuration(config: &crate::model::Configuration) -> anyhow::Result<()> {
    let path = config_file_path()?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Promote any remaining plaintext passwords to the OS credential manager
    // before clearing them from the config file.
    let mut config = config.clone();
    for server in &mut config.servers {
        if let Some(password) = server.password.take() {
            let service = "jenkins_tray";
            let account = &server.url;
            if let Ok(entry) = keyring::Entry::new(service, account) {
                let _ = entry.set_password(&password);
            }
        }
    }

    // Now write the config with passwords cleared (they're in keyring now)
    for server in &mut config.servers {
        server.password = None;
    }

    let data = serde_json::to_string_pretty(&config)?;
    std::fs::write(&path, data)?;
    Ok(())
}