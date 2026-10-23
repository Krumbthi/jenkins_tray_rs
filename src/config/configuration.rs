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
        load_passwords(&mut config);
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
                load_passwords(&mut config);
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

/// Keyring service name; the account is the server URL.
const KEYRING_SERVICE: &str = "jenkins_tray";

/// Stores a password in the OS credential manager; `false` when the backend is
/// unavailable (e.g. no secret service under WSLg).
fn store_password(account: &str, password: &str) -> bool {
    keyring::Entry::new(KEYRING_SERVICE, account)
        .and_then(|entry| entry.set_password(password))
        .is_ok()
}

/// Reads a password back from the OS credential manager.
fn keyring_password(account: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, account)
        .ok()?
        .get_password()
        .ok()
        .filter(|password| !password.is_empty())
}

/// Removes the stored password (best effort).
///
/// Called when the user clears the password field: without this, the next load
/// would restore the stale credential from the OS credential manager.
fn forget_password(account: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account) {
        let _ = entry.delete_credential();
    }
}

/// Whether the server has credentials worth looking up.
fn has_username(server: &crate::model::Server) -> bool {
    server.username.as_deref().is_some_and(|username| !username.is_empty())
}

/// Moves plaintext passwords into the OS credential manager and restores the
/// ones a previous save already moved there.
///
/// A plaintext password (from an older build, or added by the Edit Server
/// dialog, which stores it in the configuration until the next save) is only
/// dropped once the credential manager accepted it; otherwise it stays in the
/// configuration and the next save retries, instead of silently losing the
/// credentials.
///
/// An *empty* password means the user cleared the field: the stored
/// credential is deleted so it is not restored again.
fn load_passwords(config: &mut crate::model::Configuration) {
    for server in &mut config.servers {
        match server.password.take() {
            Some(password) if password.is_empty() => {
                forget_password(&server.url);
                server.password = Some(password);
            }
            Some(password) => {
                if !store_password(&server.url, &password) {
                    server.password = Some(password);
                }
            }
            None if has_username(server) => {
                server.password = keyring_password(&server.url);
            }
            None => {}
        }
    }
}

/// Drops every plaintext password that the credential manager accepted,
/// keeping the ones it refused (so they are written to the config and retried
/// on the next load). An empty password stays in the config (and deletes any
/// stored credential) so that clearing the field survives a restart.
fn promote_passwords(config: &mut crate::model::Configuration) {
    for server in &mut config.servers {
        let Some(password) = server.password.take() else {
            continue;
        };
        if password.is_empty() {
            forget_password(&server.url);
            server.password = Some(password);
        } else if !store_password(&server.url, &password) {
            server.password = Some(password);
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
/// OS credential manager (keyring) so that the JSON file does not contain
/// plaintext passwords when the manager is available. If it is not, the
/// plaintext is kept (and retried next time) rather than losing the
/// credentials.
pub fn save_configuration(config: &crate::model::Configuration) -> anyhow::Result<()> {
    let path = config_file_path()?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config = config.clone();
    promote_passwords(&mut config);

    let data = serde_json::to_string_pretty(&config)?;
    std::fs::write(&path, data)?;
    Ok(())
}