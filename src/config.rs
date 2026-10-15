/// Configuration file paths and persistence.
pub mod configuration;

use std::sync::RwLock;

use crate::model::{
    Configuration, GeneralSettings, NotificationSettings, Project, ProjectState, QueueItem, Server,
};

/// Configuration service providing in-memory access to the Jenkins Tray config.
pub struct ConfigurationService {
    config: RwLock<Configuration>,
}

impl ConfigurationService {
    /// Create a new ConfigurationService and load existing configuration.
    pub fn new() -> anyhow::Result<Self> {
        let config = configuration::load_configuration()?;
        Ok(Self {
            config: RwLock::new(config),
        })
    }

    /// Get the current configuration.
    pub fn get_config(&self) -> anyhow::Result<Configuration> {
        let c = self.config.read().map_err(|e| anyhow::anyhow!("RwLock read error: {}", e))?;
        Ok(c.clone())
    }

    /// Save the configuration to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let config = self.config.read().map_err(|e| anyhow::anyhow!("RwLock read error: {}", e))?;
        configuration::save_configuration(&config)
    }

    /// Apply a mutation to the configuration and persist the result.
    ///
    /// The write guard is always dropped before saving: [`Self::save`] takes a
    /// read lock, which would deadlock against a write guard still held by the
    /// same thread.
    fn mutate(&self, f: impl FnOnce(&mut Configuration)) -> anyhow::Result<()> {
        let snapshot = {
            let mut config = self
                .config
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock write error: {}", e))?;
            f(&mut config);
            config.clone()
        };
        configuration::save_configuration(&snapshot)
    }

    /// Remembers the parameter values submitted for a job, so the next
    /// "Build with parameters" dialog starts with them (new feature; the
    /// original has no parameter dialog). An empty map forgets the job.
    pub fn remember_build_parameters(
        &self,
        job_url: &str,
        values: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<()> {
        self.mutate(|config| {
            if values.is_empty() {
                config.last_build_parameters.remove(job_url);
            } else {
                config
                    .last_build_parameters
                    .insert(job_url.to_string(), values.clone());
            }
        })
    }

    /// Add a new Jenkins server (original: ConfigurationService.AddServer).
    pub fn add_server(
        &self,
        url: &str,
        display_name: &str,
        username: Option<&str>,
        password: Option<&str>,
        ignore_untrusted_certificate: bool,
    ) -> anyhow::Result<Server> {
        let server = Server {
            url: url.to_string(),
            display_name: display_name.to_string(),
            username: username.map(String::from),
            password: password.map(String::from),
            ignore_untrusted_certificate,
            projects: Vec::new(),
        };
        self.mutate(|config| config.servers.push(server.clone()))?;
        Ok(server)
    }

    /// Update an existing server in place (original: ConfigurationService.UpdateServer).
    ///
    /// When the URL changes, every configured project URL is rewritten to the
    /// new base URL, exactly like the original service.
    pub fn update_server(
        &self,
        old_url: &str,
        url: &str,
        display_name: &str,
        username: Option<&str>,
        password: Option<&str>,
        ignore_untrusted_certificate: bool,
    ) -> anyhow::Result<()> {
        self.mutate(|config| {
            let Some(server) = config.servers.iter_mut().find(|s| s.url == old_url) else {
                return;
            };

            let old_url = old_url.to_string();
            let new_url = url.to_string();
            server.url = new_url.clone();
            server.display_name = display_name.to_string();
            server.ignore_untrusted_certificate = ignore_untrusted_certificate;
            // original BindData: credentials are cleared when no username is given
            if !username.unwrap_or_default().is_empty() {
                server.username = username.map(String::from);
                server.password = password.map(String::from);
            } else {
                server.username = None;
                server.password = None;
            }

            if !old_url.eq_ignore_ascii_case(&new_url) {
                log::info!("Server Url updated: {} -> {}", old_url, new_url);
                for project in &mut server.projects {
                    if project.url.contains(&old_url) {
                        let updated = project.url.replacen(&old_url, &new_url, 1);
                        log::info!("Project Url updated: {} -> {}", project.url, updated);
                        project.url = updated;
                    }
                }
            }
        })
    }

    /// Remove a server from the configuration (original: ConfigurationService.RemoveServer).
    pub fn remove_server(&self, server: &Server) -> anyhow::Result<()> {
        self.mutate(|config| config.servers.retain(|s| s.url != server.url))
    }

    /// Add a project to a server's project list (original: ConfigurationService.AddProject).
    ///
    /// The original stores projects in a set, so duplicates are ignored.
    pub fn add_project(&self, project: &Project) -> anyhow::Result<()> {
        self.mutate(|config| Self::insert_project(config, project))
    }

    /// Add several projects at once, saving once
    /// (original: ConfigurationService.AddProjects / "Select all projects").
    pub fn add_projects(&self, projects: &[Project]) -> anyhow::Result<()> {
        self.mutate(|config| {
            for project in projects {
                Self::insert_project(config, project);
            }
        })
    }

    fn insert_project(config: &mut Configuration, project: &Project) {
        let Some(ref server_url) = project.server else {
            return;
        };
        if let Some(target) = config.servers.iter_mut().find(|s| s.url == *server_url)
            && !target.projects.iter().any(|p| p.url == project.url)
        {
            target.projects.push(project.clone());
        }
    }

    /// Remove a project from a server's project list (original: ConfigurationService.RemoveProject).
    pub fn remove_project(&self, project: &Project) -> anyhow::Result<()> {
        self.mutate(|config| Self::extract_project(config, project))
    }

    /// Remove several projects at once, saving once
    /// (original: ConfigurationService.RemoveProjects / "Unselect all projects").
    pub fn remove_projects(&self, projects: &[Project]) -> anyhow::Result<()> {
        self.mutate(|config| {
            for project in projects {
                Self::extract_project(config, project);
            }
        })
    }

    fn extract_project(config: &mut Configuration, project: &Project) {
        let Some(ref server_url) = project.server else {
            return;
        };
        if let Some(target) = config.servers.iter_mut().find(|s| s.url == *server_url) {
            target.projects.retain(|p| p.url != project.url);
        }
    }

    /// Store the authentication token / cause text of a project
    /// (original: AuthenticationTokenForm -> Project.AuthenticationToken / CauseText).
    pub fn set_project_authentication(
        &self,
        server_url: &str,
        project_url: &str,
        token: Option<&str>,
        cause: Option<&str>,
    ) -> anyhow::Result<()> {
        self.mutate(|config| {
            if let Some(project) = Self::find_project_mut(config, server_url, project_url) {
                project.authentication_token = token.map(String::from);
                project.cause_text = cause.map(String::from);
            }
        })
    }

    /// Set (or clear) the acknowledged flag of a project
    /// (original: Project.IsAcknowledged, toggled by "Acknowledge project").
    pub fn set_project_acknowledged(
        &self,
        server_url: &str,
        project_url: &str,
        acknowledged: bool,
    ) -> anyhow::Result<()> {
        self.mutate(|config| {
            if let Some(project) = Self::find_project_mut(config, server_url, project_url) {
                project.is_acknowledged = acknowledged;
            }
        })
    }

    fn find_project_mut<'a>(
        config: &'a mut Configuration,
        server_url: &str,
        project_url: &str,
    ) -> Option<&'a mut Project> {
        let server = config.servers.iter_mut().find(|s| s.url == server_url)?;
        server.projects.iter_mut().find(|p| p.url == project_url)
    }

    /// Apply the runtime state fetched by an update cycle to the configured
    /// projects - in memory only, never persisted.
    ///
    /// The original mutates the shared `Project` objects directly (its JSON
    /// only contains the `[JsonProperty]` fields), so a poll result must not
    /// end up in `jenkins_tray.json`.
    pub fn apply_project_states(
        &self,
        states: &[(String, ProjectState)],
    ) -> anyhow::Result<()> {
        let mut config = self
            .config
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock write error: {}", e))?;

        for (project_url, state) in states {
            let Some(project) = config
                .servers
                .iter_mut()
                .flat_map(|server| server.projects.iter_mut())
                .find(|project| project.url == *project_url)
            else {
                continue;
            };

            // original: project.DisplayName = displayName (always assigned)
            project.display_name = state.display_name.clone();

            // original: the queue fields are overwritten only when the XML
            // carried a value; InQueue is always assigned.
            let queue = project.queue.get_or_insert_with(QueueItem::default);
            queue.in_queue = state.in_queue;
            if let Some(id) = state.queue_id {
                queue.id = Some(id);
            }
            if let Some(why) = &state.queue_why {
                queue.why = Some(why.clone());
            }
            if let Some(stuck) = state.queue_stuck {
                queue.stuck = Some(stuck);
            }
            if let Some(since) = state.in_queue_since {
                queue.in_queue_since = Some(since);
            }

            // original: previous status is carried over into the new details
            if let Some(details) = &state.details {
                let previous = project
                    .all_build_details
                    .as_ref()
                    .map(|previous| previous.status.clone());
                let mut details = details.clone();
                if previous.is_some() {
                    details.previous_status = previous;
                }
                project.all_build_details = Some(details);
            }
        }
        Ok(())
    }

    /// Get all projects grouped by server.
    /// Get the general settings.
    pub fn get_general_settings(&self) -> anyhow::Result<GeneralSettings> {
        let c = self.config.read().map_err(|e| anyhow::anyhow!("RwLock read error: {}", e))?;
        Ok(c.general_settings.clone())
    }

    /// Set the general settings (original: SettingsForm "General" tab).
    pub fn set_general_settings(&self, settings: GeneralSettings) -> anyhow::Result<()> {
        self.mutate(|config| config.general_settings = settings)
    }

    /// Get the notification settings.
    pub fn get_notification_settings(&self) -> anyhow::Result<NotificationSettings> {
        let c = self.config.read().map_err(|e| anyhow::anyhow!("RwLock read error: {}", e))?;
        Ok(c.notification_settings.clone())
    }

    /// Set the notification settings (original: SettingsForm "Notifications" tab).
    pub fn set_notification_settings(&self, settings: NotificationSettings) -> anyhow::Result<()> {
        self.mutate(|config| config.notification_settings = settings)
    }

    /// Get the refresh interval in seconds.
    pub fn get_refresh_interval(&self) -> anyhow::Result<u64> {
        self.get_general_settings().map(|s| s.refresh_interval_in_seconds)
    }
}