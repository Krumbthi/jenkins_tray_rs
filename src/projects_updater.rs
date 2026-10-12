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

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Notify, Semaphore};
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

    /// Start polling (original: ProjectsUpdateService.Initialize - the timer
    /// fires immediately and then after every refresh interval).
    pub fn start(&mut self, runtime: &tokio::runtime::Handle) {
        if self.handle.is_some() {
            return;
        }
        log::info!("Starting projects update service");

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

        self.handle = Some(runtime.spawn(async move {
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
            for project in &server.projects {
                let client = self.clients.for_server(server).clone();
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
                        jenkins_client::update_project(&client, &server, &project, &throwaway)
                            .await
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
