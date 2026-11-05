//! Projects update service.
//!
//! Port of C# `ProjectsUpdateService.cs`: a loop that refreshes every
//! configured project against its Jenkins server, applies the results to the
//! configuration (in memory) and then raises the `ProjectsUpdated` event so
//! the grid, the tray notifier and the notification service can update.
//!
//! Differences from the original, noted explicitly:
//! - The C# version runs the cycle on a `BackgroundWorker` and *skips* a
//!   trigger while one is running; this loop is sequential, so overlap is
//!   impossible by construction.
//! - The refresh interval is re-read from the configuration after every
//!   cycle, like the original timer, but is clamped to at least one second.
//! - Concurrency is a single global semaphore of 8 (the original uses a pool
//!   of 8 threads with 4 per server); the original debug build uses 1.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, Timelike};
use tokio::sync::{Notify, Semaphore, watch};
use tokio::task::JoinHandle;

use crate::config::ConfigurationService;
use crate::jenkins_client::{self, Clients, SharedCache};
use crate::model::ProjectState;

/// Configuration for the polling service.
#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    /// Reuse the build-details XML cache between cycles
    /// (original: JenkinsService cache / RecycleCache).
    pub enable_cache: bool,
    /// Maximum number of parallel project refreshes
    /// (original: TOTAL_THREAD_COUNT = 8 in release builds).
    pub max_concurrent: usize,
}

impl Default for UpdaterConfig {
    fn default() -> Self {
        Self {
            enable_cache: true,
            max_concurrent: 8,
        }
    }
}

/// Callback invoked after every completed update cycle (the original
/// `ProjectsUpdated` event; subscribers are the main form, the tray notifier
/// and the notification service).
pub type ProjectsUpdatedHandler = Arc<dyn Fn() + Send + Sync>;

/// The polling service. Owned by the UI thread; the loop itself runs as a
/// tokio task.
pub struct ProjectsUpdateService {
    config: UpdaterConfig,
    clients: Clients,
    config_service: Arc<ConfigurationService>,
    cache: SharedCache,
    /// Wakes the loop for an immediate refresh (original:
    /// ProjectsUpdateService.UpdateProjects).
    wake: Arc<Notify>,
    shutdown_tx: Option<watch::Sender<bool>>,
    on_updated: Option<ProjectsUpdatedHandler>,
    handle: Option<JoinHandle<()>>,
}

impl ProjectsUpdateService {
    pub fn new(
        config: UpdaterConfig,
        clients: Clients,
        config_service: Arc<ConfigurationService>,
    ) -> Self {
        Self {
            config,
            clients,
            config_service,
            cache: jenkins_client::new_cache(),
            wake: Arc::new(Notify::new()),
            shutdown_tx: None,
            on_updated: None,
            handle: None,
        }
    }

    /// Register the `ProjectsUpdated` subscriber.
    pub fn set_on_updated(&mut self, handler: ProjectsUpdatedHandler) {
        self.on_updated = Some(handler);
    }

    /// Starts both the polling loop and the minute scheduler. The scheduler
    /// runs every minute and checks each configured project's
    /// `periodic_trigger`; when the spec matches the current minute it calls
    /// `trigger_build_parameters` like a manual run.
    pub fn start(&mut self, runtime: &tokio::runtime::Handle) {
        if self.handle.is_some() {
            return;
        }
        log::info!("Starting projects update service + periodic scheduler");

        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let loop_service = Arc::new(UpdateLoop {
            config: self.config.clone(),
            clients: self.clients.clone(),
            config_service: self.config_service.clone(),
            cache: self.cache.clone(),
            wake: self.wake.clone(),
            on_updated: self.on_updated.clone(),
        });

        let scheduler = MinuteScheduler {
            config_service: self.config_service.clone(),
            clients: self.clients.clone(),
            last_fired: HashMap::new(),
        };
        let scheduler_shutdown = shutdown_rx.clone();

        self.handle = Some(runtime.spawn(async move {
            // The cron scheduler runs on its own minute boundaries, so it does
            // not drift with the configurable refresh interval.
            let scheduler_task = tokio::spawn(async move {
                scheduler.run(scheduler_shutdown).await;
            });

            // First cycle immediately (original dueTime = 0).
            loop_service.run_cycle().await;
            loop {
                let interval = loop_service.refresh_interval();
                tokio::select! {
                    biased;
                    _ = shutdown_rx.changed() => break,
                    _ = loop_service.wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                }
                loop_service.run_cycle().await;
            }
            scheduler_task.abort();
            log::info!("Projects update loop stopped");
        }));
    }

    /// Trigger an immediate refresh (original:
    /// ProjectsUpdateService.UpdateProjects - a manual refresh / F5).
    pub fn refresh_all(&self) {
        self.wake.notify_one();
    }

    /// The shared build-details XML cache, used by the claim action to drop
    /// a document (original: `JenkinsService.RemoveFromCache`).
    pub fn cache(&self) -> SharedCache {
        self.cache.clone()
    }

    /// Stop polling.
    pub async fn stop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        log::info!("Stopping projects update service");
    }
}

/// The state captured by the polling task.
struct UpdateLoop {
    config: UpdaterConfig,
    clients: Clients,
    config_service: Arc<ConfigurationService>,
    cache: SharedCache,
    wake: Arc<Notify>,
    on_updated: Option<ProjectsUpdatedHandler>,
}

/// One minute scheduler that triggers periodic builds for jobs with a
/// `periodic_trigger` spec (cron + optional parameters).
struct MinuteScheduler {
    config_service: Arc<ConfigurationService>,
    clients: Clients,
    /// Last minute in which a schedule fired, keyed by server + project URL.
    /// Keeps a delayed tick or a clock adjustment from starting the same
    /// build twice within one minute.
    last_fired: HashMap<String, String>,
}

impl UpdateLoop {
    /// original: timer reschedule uses
    /// `GeneralSettings.RefreshIntervalInSeconds`.
    fn refresh_interval(&self) -> u64 {
        self.config_service
            .get_refresh_interval()
            .unwrap_or(15)
            .max(1)
    }

    /// One full refresh of all configured projects
    /// (original: DoUpdateProjectsInternal).
    async fn run_cycle(&self) {
        log::info!("Running update from Timer");

        let config = match self.config_service.get_config() {
            Ok(config) => config,
            Err(e) => {
                log::error!("Could not read the configuration: {e:#}");
                return;
            }
        };

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent.max(1)));
        let mut join_set = tokio::task::JoinSet::new();

        for server in &config.servers {
            // One shared snapshot per server: cloning the whole `Server` for
            // every project would copy its complete project list once per
            // project, i.e. quadratically in the number of projects.
            let server = Arc::new(server.clone());
            for project in &server.projects {
                let client = self.clients.for_server(&server).clone();
                let server = server.clone();
                let project = project.clone();
                let cache = self.cache.clone();
                let semaphore = semaphore.clone();
                let enable_cache = self.config.enable_cache;
                join_set.spawn(async move {
                    let url = project.url.clone();
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(permit) => permit,
                        Err(_) => return (url, Err(anyhow::anyhow!("updater stopped"))),
                    };
                    let result = if enable_cache {
                        jenkins_client::update_project(&client, &server, &project, &cache).await
                    } else {
                        // Without the cache every project gets a throwaway one.
                        let throwaway = jenkins_client::new_cache();
                        jenkins_client::update_project(&client, &server, &project, &throwaway).await
                    };
                    (url, result)
                });
            }
        }

        let mut states: Vec<(String, ProjectState)> = Vec::new();
        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok((url, Ok(state))) => states.push((url, state)),
                // original: errors are logged per project and leave the
                // project's previous details untouched
                Ok((url, Err(e))) => {
                    log::error!("Error updating {url}: {}", crate::errors::root_cause(&e))
                }
                Err(e) => log::error!("Update task failed: {e}"),
            }
        }

        if let Err(e) = self.config_service.apply_project_states(&states) {
            log::error!("Could not apply the project updates: {e:#}");
        }
        if self.config.enable_cache {
            jenkins_client::recycle_cache(&self.cache);
        }

        log::info!("Done");

        // original: ProjectsUpdated event
        if let Some(handler) = &self.on_updated {
            handler();
        }
    }
}

impl MinuteScheduler {
    /// Waits for the next minute boundary and then triggers every matching
    /// project. Mirrors Jenkins' client-side "build periodically" behavior:
    /// only runs while the application is running, without catching up.
    async fn run(mut self, mut shutdown_rx: watch::Receiver<bool>) {
        loop {
            let now = Local::now();
            let into_minute = now.second() as u64 * 1_000_000_000 + now.nanosecond() as u64;
            let delay = Duration::from_nanos(60_000_000_000u64.saturating_sub(into_minute).max(1));
            tokio::select! {
                biased;
                _ = shutdown_rx.changed() => break,
                _ = tokio::time::sleep(delay) => {}
            }
            if *shutdown_rx.borrow() {
                break;
            }
            self.tick().await;
        }
        log::info!("Periodic build scheduler stopped");
    }

    /// Triggers a parameterized build for every project whose
    /// `periodic_trigger` spec matches the current minute.
    async fn tick(&mut self) {
        let config = match self.config_service.get_config() {
            Ok(config) => config,
            Err(e) => {
                log::error!("Could not read configuration for scheduler: {e:#}");
                return;
            }
        };
        let now = Local::now();
        let stamp = now.format("%Y-%m-%d %H:%M").to_string();
        let mut join_set = tokio::task::JoinSet::new();

        for server in &config.servers {
            let server = Arc::new(server.clone());
            for project in &server.projects {
                let Some(spec) = project.periodic_trigger.as_deref() else {
                    continue;
                };
                let (cron, params) = jenkins_client::parse_periodic_trigger(spec);
                if cron.is_empty() || !cron_matches(&cron, &project.name, &now) {
                    continue;
                }
                let key = format!("{}\n{}", server.url, project.url);
                if self.last_fired.get(&key).is_some_and(|last| last == &stamp) {
                    continue;
                }
                self.last_fired.insert(key, stamp.clone());

                let client = self.clients.for_server(&server).clone();
                let server = server.clone();
                let project = project.clone();
                let name = project.name.clone();
                let cron = cron.clone();
                log::info!("Periodic trigger fired for {name} (schedule {cron})");
                join_set.spawn(async move {
                    if let Err(e) = jenkins_client::trigger_build_parameters(
                        &client, &server, &project, &params,
                    )
                    .await
                    {
                        let cause = crate::errors::root_cause(&e);
                        log::error!("Periodic build of {name} failed: {cause}");
                    } else {
                        log::info!("Periodic build triggered for {name}");
                    }
                });
            }
        }

        while let Some(joined) = join_set.join_next().await {
            if let Err(e) = joined {
                log::error!("Periodic build task failed: {e}");
            }
        }
        // Only the current minute matters; drop earlier entries so the map
        // cannot grow with removed projects.
        self.last_fired.retain(|_, fired| fired == &stamp);
    }
}

// ---------------------------------------------------------------------------
// Jenkins cron matching (for the client-side periodic trigger)
// ---------------------------------------------------------------------------

const MONTH_NAMES: &[(&str, u32)] = &[
    ("JAN", 1),
    ("FEB", 2),
    ("MAR", 3),
    ("APR", 4),
    ("MAY", 5),
    ("JUN", 6),
    ("JUL", 7),
    ("AUG", 8),
    ("SEP", 9),
    ("OCT", 10),
    ("NOV", 11),
    ("DEC", 12),
];

const DAY_NAMES: &[(&str, u32)] = &[
    ("SUN", 0),
    ("MON", 1),
    ("TUE", 2),
    ("WED", 3),
    ("THU", 4),
    ("FRI", 5),
    ("SAT", 6),
];

const NO_NAMES: &[(&str, u32)] = &[];

/// One parsed comma-separated item of a cron field.
struct CronItem {
    atom: CronAtom,
    /// `None` means a single value; otherwise the item is a progression.
    step: Option<u32>,
}

enum CronAtom {
    Single(u32),
    Range(u32, u32),
    /// A bare `H`: a stable hash offset within the field's range.
    Hash(u32, u32),
    /// `H(a-b)`: a stable hash offset within `a..=b`.
    HashRange(u32, u32),
}

/// Expands the `@...` aliases Jenkins accepts to a five-field expression.
fn canonical_cron(spec: &str) -> Option<String> {
    let spec = spec.trim();
    let alias = match spec.to_ascii_lowercase().as_str() {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        _ => spec,
    };
    if alias.split_whitespace().count() != 5 {
        return None;
    }
    Some(alias.to_string())
}

fn parse_value(token: &str, names: &[(&str, u32)]) -> Option<u32> {
    if let Ok(value) = token.parse() {
        return Some(value);
    }
    let upper = token.to_ascii_uppercase();
    names
        .iter()
        .find(|(name, _)| *name == upper)
        .map(|(_, value)| *value)
}

/// Parses one comma-separated item. `step` replaces `a/step` with the range
/// `a..=max`, matching the usual cron meaning.
fn parse_item(item: &str, min: u32, max: u32, names: &[(&str, u32)]) -> Option<CronItem> {
    let (base, step) = match item.split_once('/') {
        Some((base, step)) => {
            let step = step.trim().parse::<u32>().ok()?;
            if step == 0 {
                return None;
            }
            (base.trim(), Some(step))
        }
        None => (item.trim(), None),
    };
    if base.is_empty() {
        return None;
    }

    let atom = if base == "*" {
        CronAtom::Range(min, max)
    } else if base == "H" {
        if step.is_some() {
            CronAtom::HashRange(min, max)
        } else {
            CronAtom::Hash(min, max)
        }
    } else if let Some(inner) = base
        .strip_prefix("H(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let (a, b) = inner.split_once('-')?;
        let a = parse_value(a.trim(), names)?;
        let b = parse_value(b.trim(), names)?;
        if a < min || b > max || a > b {
            return None;
        }
        CronAtom::HashRange(a, b)
    } else if let Some((a, b)) = base.split_once('-') {
        let a = parse_value(a.trim(), names)?;
        let b = parse_value(b.trim(), names)?;
        if a < min || b > max || a > b {
            return None;
        }
        CronAtom::Range(a, b)
    } else {
        let value = parse_value(base, names)?;
        if value < min || value > max {
            return None;
        }
        if step.is_some() {
            CronAtom::Range(value, max)
        } else {
            CronAtom::Single(value)
        }
    };

    Some(CronItem { atom, step })
}

/// Validates a Jenkins cron expression (five fields, `H` tokens and the
/// `@hourly`/`@daily`/... aliases). Used by the "Build with parameters"
/// dialog before a schedule is accepted.
pub fn validate_cron_spec(spec: &str) -> Result<(), String> {
    let cron = canonical_cron(spec).ok_or_else(|| {
        "expected five fields: minute hour day-of-month month day-of-week".to_string()
    })?;
    let fields: Vec<&str> = cron.split_whitespace().collect();
    let definitions: [(&str, u32, u32, &[(&str, u32)]); 5] = [
        ("minute", 0, 59, NO_NAMES),
        ("hour", 0, 23, NO_NAMES),
        ("day-of-month", 1, 31, NO_NAMES),
        ("month", 1, 12, MONTH_NAMES),
        ("day-of-week", 0, 7, DAY_NAMES),
    ];
    for (field, (label, min, max, names)) in fields.iter().zip(definitions) {
        let valid = !field.trim().is_empty()
            && field
                .split(',')
                .all(|item| parse_item(item.trim(), min, max, names).is_some());
        if !valid {
            return Err(format!("invalid {label} field: \"{field}\""));
        }
    }
    Ok(())
}

/// Stable per-field hash used for Jenkins `H` tokens. Real Jenkins hashes the
/// job name with its own algorithm; any stable hash gives the same property
/// here (runs are spread out and keep their offset across restarts).
fn hash_for(job_name: &str, field: usize) -> u64 {
    let field_tag = field.to_string();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in job_name
        .bytes()
        .chain(std::iter::once(b'#'))
        .chain(field_tag.bytes())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn item_matches(item: &CronItem, value: u32, hash: u64) -> bool {
    let in_progression = |first: u32, last: u32, step: u32| {
        value >= first && value <= last && (value - first) % step == 0
    };
    match item.atom {
        CronAtom::Single(expected) => item.step.is_none() && value == expected,
        CronAtom::Range(first, last) => in_progression(first, last, item.step.unwrap_or(1)),
        CronAtom::Hash(min, max) => {
            let span = max - min + 1;
            value == min + (hash % span as u64) as u32
        }
        CronAtom::HashRange(first, last) => match item.step {
            Some(step) => {
                // Like Jenkins `H/step`, the first run is somewhere within the
                // step; fall back into the range when the step is larger.
                let mut start = first + (hash % step as u64) as u32;
                if start > last {
                    start = first + (hash % (last - first + 1) as u64) as u32;
                }
                in_progression(start, last, step)
            }
            None => {
                let span = last - first + 1;
                value == first + (hash % span as u64) as u32
            }
        },
    }
}

fn field_matches(
    field: &str,
    value: u32,
    min: u32,
    max: u32,
    hash: u64,
    names: &[(&str, u32)],
) -> bool {
    field.split(',').any(|item| {
        parse_item(item.trim(), min, max, names)
            .is_some_and(|item| item_matches(&item, value, hash))
    })
}

/// Whether `spec` matches `now`. Supports `*`, lists, ranges, steps, `H`,
/// `H(a-b)`, `H/step`, `H(a-b)/step`, names and the `@...` aliases. Day of
/// month and day of week use the usual cron OR rule when both are restricted.
fn cron_matches(spec: &str, job_name: &str, now: &DateTime<Local>) -> bool {
    let Some(cron) = canonical_cron(spec) else {
        return false;
    };
    let fields: Vec<&str> = cron.split_whitespace().collect();
    let minute = field_matches(
        fields[0],
        now.minute(),
        0,
        59,
        hash_for(job_name, 0),
        NO_NAMES,
    );
    let hour = field_matches(
        fields[1],
        now.hour(),
        0,
        23,
        hash_for(job_name, 1),
        NO_NAMES,
    );
    let day_of_month = field_matches(fields[2], now.day(), 1, 31, hash_for(job_name, 2), NO_NAMES);
    let month = field_matches(
        fields[3],
        now.month(),
        1,
        12,
        hash_for(job_name, 3),
        MONTH_NAMES,
    );
    let weekday = now.weekday().num_days_from_sunday();
    let day_of_week_matches =
        |value: u32| field_matches(fields[4], value, 0, 7, hash_for(job_name, 4), DAY_NAMES);
    let day_of_week = day_of_week_matches(weekday) || (weekday == 0 && day_of_week_matches(7));

    let dom_restricted = !fields[2].trim_start().starts_with('*');
    let dow_restricted = !fields[4].trim_start().starts_with('*');
    let day = if dom_restricted && dow_restricted {
        day_of_month || day_of_week
    } else {
        day_of_month && day_of_week
    };

    minute && hour && month && day
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        // A fixed weekday in the middle of a month so day/month wildcards and
        // the day-of-week tests are deterministic.
        Local
            .with_ymd_and_hms(2026, 7, 15, hour, minute, 0)
            .single()
            .expect("valid local time")
    }

    #[test]
    fn wildcard_matches_every_minute() {
        assert!(cron_matches("* * * * *", "job", &at(13, 37)));
    }

    #[test]
    fn specific_time_matches_only_that_minute() {
        assert!(cron_matches("30 14 * * *", "job", &at(14, 30)));
        assert!(!cron_matches("30 14 * * *", "job", &at(14, 31)));
        assert!(!cron_matches("30 14 * * *", "job", &at(13, 30)));
    }

    #[test]
    fn steps_and_ranges() {
        assert!(cron_matches("*/15 * * * *", "job", &at(10, 45)));
        assert!(!cron_matches("*/15 * * * *", "job", &at(10, 46)));
        assert!(cron_matches("0 8-17 * * *", "job", &at(17, 0)));
        assert!(!cron_matches("0 8-17 * * *", "job", &at(18, 0)));
    }

    #[test]
    fn hash_tokens_are_stable_and_in_range() {
        let matches: Vec<u32> = (0..60)
            .filter(|minute| cron_matches("H * * * *", "job", &at(10, *minute)))
            .collect();
        assert_eq!(matches.len(), 1, "H must select exactly one minute");
        // Calling it again yields the same minute (stable across restarts).
        assert!(cron_matches("H * * * *", "job", &at(10, matches[0])));
    }

    #[test]
    fn day_of_month_or_day_of_week() {
        // 2026-07-15 is a Wednesday (weekday 3).
        assert!(cron_matches("0 0 15 * *", "job", &at(0, 0)));
        assert!(cron_matches("0 0 * * 3", "job", &at(0, 0)));
        // Both restricted: either one matching is enough.
        assert!(cron_matches("0 0 1 * 3", "job", &at(0, 0)));
        // Only day of month restricted: day of week must also hold.
        assert!(!cron_matches("0 0 1 * *", "job", &at(0, 0)));
    }

    #[test]
    fn aliases_and_validation() {
        assert!(validate_cron_spec("@daily").is_ok());
        assert!(validate_cron_spec("H H * * *").is_ok());
        assert!(validate_cron_spec("H(0-29)/10 * * * *").is_ok());
        assert!(validate_cron_spec("*/15 8-17 * JAN-MAR MON-FRI").is_ok());
        assert!(validate_cron_spec("61 * * * *").is_err());
        assert!(validate_cron_spec("* * * *").is_err());
        assert!(validate_cron_spec("1,,2 * * * *").is_err());
        assert!(validate_cron_spec("H(10-5) * * * *").is_err());
    }
}
