/// Config file name (renamed from the original C# `jenkins.configuration`).
pub const CONFIGURATION_FILE: &str = "jenkins_tray.json";

/// Previous configuration file name, used by the original C# application and
/// by earlier builds of this port. Still read once for migration.
pub const LEGACY_CONFIGURATION_FILE: &str = "jenkins.configuration";

/// Directory name for Jenkins Tray app data (mirrors C# JENKINS_TRAY_DIRECTORY)
pub const JENKINS_TRAY_DIRECTORY: &str = "Jenkins Tray";

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

/// Load configuration from disk.
///
/// If `jenkins_tray.json` does not exist yet, an older
/// `jenkins.configuration` file is migrated (read and re-saved under the new
/// name). Files that cannot be parsed (e.g. one written by the original C#
/// application, whose format differs - see the deviation note on
/// [`crate::model::Configuration`]) are ignored with a warning instead of
/// aborting the startup.
pub fn load_configuration() -> anyhow::Result<crate::model::Configuration> {
    let path = config_file_path()?;

    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let config: crate::model::Configuration = serde_json::from_str(&data)?;
        return Ok(normalize(config));
    }

    // One-time migration from the previous file name.
    let legacy_path = config_file_path_for(LEGACY_CONFIGURATION_FILE)?;
    if legacy_path.exists() {
        let legacy: Option<crate::model::Configuration> = std::fs::read_to_string(&legacy_path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok());
        match legacy {
            Some(config) => {
                log::info!(
                    "Migrating configuration from {} to {}",
                    legacy_path.display(),
                    path.display()
                );
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
pub fn save_configuration(config: &crate::model::Configuration) -> anyhow::Result<()> {
    let path = config_file_path()?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app data directory must resolve on every supported platform
    /// (regression: Windows failed because only `APP_DATA`/`HOME` were checked
    /// while Windows exports `APPDATA`), and the file name is fixed to
    /// `jenkins_tray.json`.
    #[test]
    fn config_file_path_is_resolvable() {
        let path = config_file_path()
            .expect("app data directory must be resolvable on all supported platforms");
        assert_eq!(path.file_name().unwrap(), "jenkins_tray.json");
        assert_eq!(
            path.parent().unwrap().file_name().unwrap(),
            JENKINS_TRAY_DIRECTORY
        );
    }

    /// The general-settings defaults mirror the original
    /// `GeneralSettings()` constructor (15 s / check for updates), and a
    /// zero refresh interval written by an earlier build is corrected on
    /// load while configured values are kept.
    #[test]
    fn refresh_interval_defaults_and_normalization() {
        let defaults = crate::model::GeneralSettings::default();
        assert_eq!(defaults.refresh_interval_in_seconds, 15);
        assert!(defaults.check_for_updates);

        // Missing general_settings object -> constructed defaults.
        let config: crate::model::Configuration =
            serde_json::from_str(r#"{"servers":[]}"#).expect("minimal config parses");
        assert_eq!(config.general_settings.refresh_interval_in_seconds, 15);

        let config: crate::model::Configuration = serde_json::from_str(
            r#"{"general_settings":{"refresh_interval_in_seconds":0}}"#,
        )
        .expect("config parses");
        let mut config = normalize(config);
        assert_eq!(config.general_settings.refresh_interval_in_seconds, 15);

        config.general_settings.refresh_interval_in_seconds = 30;
        let config = normalize(config);
        assert_eq!(config.general_settings.refresh_interval_in_seconds, 30);
    }
}