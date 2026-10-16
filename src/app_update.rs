//! Application update service.
//!
//! Port of C# `BusinessComponents/ApplicationUpdateService.cs` and
//! `Utils/ApplicationUpdateHandler.cs`: a timer (first run 5 seconds after
//! start-up, then hourly) plus the manual toolbar check download
//! `scripts/version.properties` from this project's repository and
//! compare its `version.number` against this build's version. When a newer
//! version exists the [`ApplicationUpdateService::on_new_version`] callback
//! fires, which shows the Yes/No prompt; accepting opens the installer page
//! and exits the application (original: `ApplicationUpdateHandler`).
//!
//! Documented deviations:
//! - The original runs the manual (`User`) check synchronously on the UI
//!   thread (`CheckForUpdates_Synchronous`); the port runs every check on the
//!   tokio runtime and posts the result message boxes back to the UI thread,
//!   so the window never blocks.
//! - The original builds `System.Version` from the feed and compares it
//!   against the assembly `FileVersion`; the port compares 4-part tuples built
//!   from `CARGO_PKG_VERSION` (missing parts count as 0), which yields the
//!   same ordering for the feed's `1.0.5.0`.
//! - `PropertiesFile.ReadProperties` supports the full Java properties
//!   syntax; the port parses plain `key=value` / `key:value` lines, which is
//!   all the feed contains.
//! - The feed is this repository's own `scripts/version.properties` (the
//!   original project's feed advertises the original C# application, which
//!   would be offered as an "update" to this fork); every check can be
//!   hard-disabled again with [`UPDATE_FEED_AVAILABLE`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::ConfigurationService;

/// Update feed (original: `ApplicationUpdateService.URL`, pointed at this
/// repository's own feed - the original C# project's feed is *not* used,
/// see [`UPDATE_FEED_AVAILABLE`]).
/// Update feed URL - GitHub raw URL for this repo's version.properties.
/// When pushing a release, update scripts/version.properties and push to master.
const URL: &str = "https://raw.githubusercontent.com/Krumbthi/jenkins_tray_rs/master/scripts/version.properties";

/// Kill switch for every update check (startup, timer and the manual
/// toolbar action); each call site logs one line and skips the check.
///
/// The feed ([`URL`]) is this repository's own `scripts/version.properties`,
/// so with the flag at `true` the checks run like the original C# app's
/// (`DoCheckForUpdates`). Set to `false` to hard-disable them again (the
/// port's state before the feed existed: the original project's feed would
/// advertise the C# application as an "update").
///
/// Currently `false`: the update mechanism is disabled for now (temporary).
/// Flip back to `true` to re-enable the startup/timer/manual checks.
const UPDATE_FEED_AVAILABLE: bool = false;

/// 5 seconds delay for the 1st check on application start up
/// (original: `DEFAULT_UPDATE_DELAY`).
const UPDATE_DELAY: Duration = Duration::from_secs(5);

/// Every hour (original: `DEFAULT_UPDATE_PERIOD`).
const UPDATE_PERIOD: Duration = Duration::from_secs(60 * 60);

/// The two property keys read from the feed (original:
/// `PROPERTY_VERSION_NUMBER` / `PROPERTY_INSTALLER_URL`).
const PROPERTY_VERSION_NUMBER: &str = "version.number";
const PROPERTY_INSTALLER_URL: &str = "version.installerUrl";

/// Who triggered a check (original: `ApplicationUpdateService.UpdateSource`).
/// The original also declares `Program` but never uses it, so it is not
/// ported (dead code, like `stopAcknowledgingMenuItem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateSource {
    /// Manual check from the toolbar (original: `UpdateSource.User`).
    User,
    /// Scheduled check from the timer (original: `UpdateSource.Timer`).
    Timer,
}

impl std::fmt::Display for UpdateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateSource::User => write!(f, "User"),
            UpdateSource::Timer => write!(f, "Timer"),
        }
    }
}

/// A 4-part version like .NET's `System.Version` (feed value: `1.0.5.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u32, u32, u32, u32);

impl Version {
    /// Parses `major[.minor[.build[.revision]]]` (original:
    /// `new Version(string)`, which requires 2-4 components and throws a
    /// `FormatException` otherwise).
    fn parse(text: &str) -> Result<Self> {
        let parts: Vec<&str> = text.trim().split('.').collect();
        if !(2..=4).contains(&parts.len()) {
            bail!("Version string '{text}' is not in major.minor[.build[.revision]] format");
        }
        let mut nums = [0u32; 4];
        for (slot, part) in nums.iter_mut().zip(parts) {
            *slot = part
                .trim()
                .parse()
                .with_context(|| format!("Version component '{part}' is not a number"))?;
        }
        Ok(Version(nums[0], nums[1], nums[2], nums[3]))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0, self.1, self.2, self.3)
    }
}

/// The `NewVersionAvailable` subscriber (original: the C#
/// `NewVersionAvailable` event delegate taking version + installer URL).
type NewVersionHandler = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// The running update service (original: the Spring-managed
/// `ApplicationUpdateService` singleton).
///
/// The timer lives in a tokio task (original: `System.Threading.Timer`) whose
/// first tick fires [`UPDATE_DELAY`] after it was last enabled and then every
/// [`UPDATE_PERIOD`], mirroring `EnableTimer`.
pub struct ApplicationUpdateService {
    runtime: tokio::runtime::Handle,
    config: Arc<ConfigurationService>,
    client: reqwest::Client,
    /// Original: `timerEnabled` (only written by `EnableTimer` transitions).
    timer_enabled: AtomicBool,
    /// Bumped whenever the timer is restarted or stopped; the timer task
    /// exits as soon as its generation is stale (original: the `Timer.Change`
    /// calls inside `EnableTimer`).
    timer_generation: AtomicU64,
    /// Original: the `updating` re-entrancy guard inside `DoCheckForUpdates`.
    updating: AtomicBool,
    /// Original: the `NewVersionAvailable` event subscribed by
    /// `ApplicationUpdateHandler`.
    on_new_version: RwLock<Option<NewVersionHandler>>,
}

impl ApplicationUpdateService {
    /// Creates the service (original: the Spring container instantiating it).
    /// Call [`ApplicationUpdateService::on_new_version`] and then
    /// [`ApplicationUpdateService::initialize`], mirroring the two Spring
    /// `init-method`s (`ApplicationUpdateService.Initialize` and
    /// `ApplicationUpdateHandler.Initialize`).
    pub fn new(runtime: tokio::runtime::Handle, config: Arc<ConfigurationService>) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            config,
            client: reqwest::Client::new(),
            timer_enabled: AtomicBool::new(false),
            timer_generation: AtomicU64::new(0),
            updating: AtomicBool::new(false),
            on_new_version: RwLock::new(None),
        })
    }

    /// Registers the `NewVersionAvailable` handler.
    pub fn on_new_version(&self, handler: NewVersionHandler) {
        match self.on_new_version.write() {
            Ok(mut slot) => *slot = Some(handler),
            Err(poisoned) => *poisoned.into_inner() = Some(handler),
        }
    }

    /// Starts the check timer according to the configuration (original:
    /// `Initialize()` creating the `Timer` and calling
    /// `EnableTimer(GeneralSettings.CheckForUpdates)`).
    pub fn initialize(self: Arc<Self>) {
        if !UPDATE_FEED_AVAILABLE {
            log::info!("Update checks disabled (UPDATE_FEED_AVAILABLE is off)");
            return;
        }
        let check_for_updates = match self.config.get_config() {
            Ok(config) => config.general_settings.check_for_updates,
            Err(e) => {
                log::error!("Could not read the update settings: {e}");
                true
            }
        };
        if check_for_updates {
            log::info!(
                "Update checks enabled: first check in {}s, then hourly",
                UPDATE_DELAY.as_secs()
            );
        } else {
            log::info!("Update checks are disabled in settings");
        }
        self.enable_timer(check_for_updates);
    }

    /// Enables or disables the scheduled checks (original: `EnableTimer`).
    /// Only state transitions do anything; re-enabling restarts the 5-second
    /// initial delay like `Timer.Change(DEFAULT_UPDATE_DELAY, ...)`.
    pub fn enable_timer(self: Arc<Self>, enable: bool) {
        if enable && !UPDATE_FEED_AVAILABLE {
            log::info!("Scheduled update checks disabled (UPDATE_FEED_AVAILABLE is off)");
            return;
        }
        let was_enabled = self.timer_enabled.swap(enable, Ordering::SeqCst);
        if was_enabled == enable {
            return; // original: EnableTimer only acts on a state change
        }
        if !enable {
            self.stop_timer();
            log::info!("Scheduled update checks disabled");
            return;
        }

        let generation = self.timer_generation.fetch_add(1, Ordering::SeqCst) + 1;
        log::info!("Scheduled update checks enabled");
        let this = self.clone();
        self.runtime.spawn(async move {
            let mut delay = UPDATE_DELAY;
            loop {
                tokio::time::sleep(delay).await;
                delay = UPDATE_PERIOD;
                if this.timer_generation.load(Ordering::SeqCst) != generation
                    || !this.timer_enabled.load(Ordering::SeqCst)
                {
                    break; // EnableTimer(false) or a restart elsewhere
                }
                // original: the timer callback swallows every error
                let _ = this.check(UpdateSource::Timer).await;
                if this.timer_generation.load(Ordering::SeqCst) != generation {
                    break; // an update was found (the original stops the timer)
                }
            }
        });
    }

    /// Stops the scheduled checks (original: `EnableTimer(false)`).
    fn stop_timer(&self) {
        self.timer_enabled.store(false, Ordering::SeqCst);
        self.timer_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Runs one update check (original: `DoCheckForUpdates`). Returns `true`
    /// when an update was found; the `on_new_version` callback has fired by
    /// then. Errors propagate to the caller (the original logs them with
    /// `LoggingHelper.LogError` and rethrows; the timer callback ignores them).
    pub async fn check(&self, source: UpdateSource) -> Result<bool> {
        if !UPDATE_FEED_AVAILABLE {
            // The manual action still answers "No update available." below
            // (original: DoCheckForUpdates hits the C# project's feed).
            log::info!("Update check from {source} skipped (UPDATE_FEED_AVAILABLE is off)");
            return Ok(false);
        }
        let check_for_updates = match self.config.get_config() {
            Ok(config) => config.general_settings.check_for_updates,
            Err(e) => {
                log::error!("Could not read the update settings: {e}");
                true
            }
        };
        if source == UpdateSource::Timer && !check_for_updates {
            log::info!("Update check is already disabled in settings; stopping timer, from {source}");
            self.stop_timer();
            return Ok(false);
        }

        log::info!("Running update check from {source}");
        if self.updating.swap(true, Ordering::SeqCst) {
            log::info!("Already in update: skipping");
            return Ok(false);
        }
        let result = self.check_internal().await;
        self.updating.store(false, Ordering::SeqCst);
        match result {
            Ok(found) => {
                log::info!("Done");
                Ok(found)
            }
            Err(e) => {
                log::error!("Update check failed: {e}");
                Err(e)
            }
        }
    }

    /// Original: `DoCheckForUpdatesInternal`.
    async fn check_internal(&self) -> Result<bool> {
        log::info!("Checking for updates from {URL}");

        let body = self
            .client
            .get(URL)
            .send()
            .await
            .with_context(|| format!("Could not download {URL}"))?
            .error_for_status()
            .with_context(|| format!("Update feed {URL} returned an error"))?
            .text()
            .await
            .with_context(|| format!("Could not read the update feed {URL}"))?;

        // original: properties[PROPERTY_*] - missing keys raise a
        // KeyNotFoundException
        let properties = read_properties(&body);
        let version_str = properties
            .get(PROPERTY_VERSION_NUMBER)
            .with_context(|| format!("Missing property: {PROPERTY_VERSION_NUMBER}"))?;
        let installer_url = properties
            .get(PROPERTY_INSTALLER_URL)
            .with_context(|| format!("Missing property: {PROPERTY_INSTALLER_URL}"))?;

        let version = Version::parse(version_str)?;
        let current_version = current_version();
        log::info!("Current version: {current_version}");
        log::info!("Last version: {version}");
        log::info!("Installer URL: {installer_url}");

        if version <= current_version {
            log::info!("No updates");
            return Ok(false);
        }

        log::info!("An update is available");
        // original: disable the timer, then raise NewVersionAvailable
        self.stop_timer();
        let handler = match self.on_new_version.read() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(handler) = handler {
            handler(version_str, installer_url);
        }
        Ok(true)
    }
}

/// This build's version (original: the assembly `FileVersion`, e.g.
/// `1.0.5.0`).
fn current_version() -> Version {
    // The package version always has at least major.minor.patch (3 parts),
    // so this cannot fail.
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is not a valid version")
}

/// Minimal `key=value` / `key:value` parser for the feed (original:
/// `PropertiesFile.ReadProperties`).
fn read_properties(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('!'))
        .filter_map(|line| {
            let separator = line.find(['=', ':'])?;
            let key = line[..separator].trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), line[separator + 1..].trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parses_with_missing_parts_as_zero() {
        assert_eq!(Version::parse("1.0.5.0").unwrap(), Version(1, 0, 5, 0));
        assert_eq!(Version::parse("1.0.5").unwrap(), Version(1, 0, 5, 0));
        assert_eq!(Version::parse(" 2.1 ").unwrap(), Version(2, 1, 0, 0));
    }

    #[test]
    fn version_ordering_matches_dotnet_semantics() {
        assert!(Version::parse("1.0.6.0").unwrap() > Version::parse("1.0.5.0").unwrap());
        assert!(Version::parse("1.0.5").unwrap() == Version::parse("1.0.5.0").unwrap());
        assert!(Version::parse("1.10.0.0").unwrap() > Version::parse("1.9.0.0").unwrap());
    }

    #[test]
    fn version_rejects_invalid_strings() {
        assert!(Version::parse("1").is_err()); // .NET requires 2-4 parts
        assert!(Version::parse("a.b").is_err());
        assert!(Version::parse("").is_err());
        assert!(Version::parse("1.2.3.4.5").is_err());
    }

    #[test]
    fn properties_parse_feed_shape() {
        let feed = "# a comment\n\
                    version.number=1.0.5.0\n\
                    version.installerUrl=https://example.test/setup.msi\n";
        let properties = read_properties(feed);
        assert_eq!(
            properties.get("version.number").map(String::as_str),
            Some("1.0.5.0")
        );
        assert_eq!(
            properties.get("version.installerUrl").map(String::as_str),
            Some("https://example.test/setup.msi")
        );
    }

    #[test]
    fn current_version_is_the_package_version_as_four_parts() {
        // `current_version()` is the port's version - the equivalent of the
        // original C# app's assembly FileVersion (e.g. `1.0.5.0`) - rendered
        // as major.minor.patch.revision like the feed's `1.0.5.0`.
        assert_eq!(
            current_version().to_string(),
            format!("{}.0", env!("CARGO_PKG_VERSION"))
        );
    }
}
