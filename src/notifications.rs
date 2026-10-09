//! Notifications.
//!
//! Port of C# `AllServersStatus.cs` + `NotificationService.cs`: after every
//! configuration or projects update the buckets (failing / still failing /
//! succeeding / fixed) are recomputed and the matching sound is played.

use std::sync::Mutex;

use anyhow::Result;

use crate::model::{BuildStatus, BuildStatusEnum, Configuration, Project, Server};

/// Trait for platform-specific notification display.
pub trait TrayNotifier: Send + Sync {
    /// Play a sound file.
    fn play_sound(&self, path: &str) -> Result<()>;
}

/// Bucket classification of all projects after one update
/// (original: `AllServersStatus`).
pub struct AllServersStatus {
    /// Last status seen per project URL (original: lastStatuses)
    last_statuses: std::collections::HashMap<String, BuildStatus>,
    /// Last *completed* status per project URL
    /// (original: lastCompletedStatuses)
    last_completed_statuses: std::collections::HashMap<String, BuildStatus>,

    /// Projects that just started failing (original: FailingProjects)
    pub failing_projects: Vec<Project>,
    /// Projects that failed again (original: StillFailingProjects)
    pub still_failing_projects: Vec<Project>,
    /// Projects that just succeeded (original: SucceedingProjects)
    pub succeeding_projects: Vec<Project>,
    /// Projects that just got fixed (original: FixedProjects)
    pub fixed_projects: Vec<Project>,
}

impl Default for AllServersStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl AllServersStatus {
    pub fn new() -> Self {
        Self {
            last_statuses: Default::default(),
            last_completed_statuses: Default::default(),
            failing_projects: Vec::new(),
            still_failing_projects: Vec::new(),
            succeeding_projects: Vec::new(),
            fixed_projects: Vec::new(),
        }
    }

    /// Reclassifies every project (original: AllServersStatus.Update).
    pub fn update(&mut self, servers: &[Server]) {
        self.failing_projects.clear();
        self.still_failing_projects.clear();
        self.succeeding_projects.clear();
        self.fixed_projects.clear();

        for server in servers {
            for project in &server.projects {
                self.update_for_project(project);

                let status = project.status();
                self.last_statuses.insert(project.url.clone(), status.clone());
                if !status.is_in_progress {
                    self.last_completed_statuses.insert(project.url.clone(), status);
                }
            }
        }
    }

    /// A project only lands in a bucket when a *previous* build was in
    /// progress - i.e. a build has just completed (original:
    /// AllServersStatus.UpdateForProject).
    fn update_for_project(&mut self, project: &Project) {
        let Some(last_status) = self.last_statuses.get(&project.url).cloned() else {
            return;
        };
        let Some(last_completed) = self.last_completed_statuses.get(&project.url).cloned() else {
            return;
        };
        if !last_status.is_in_progress {
            return;
        }

        let status = project.status();
        if status.value == BuildStatusEnum::Successful {
            if last_completed.value == BuildStatusEnum::Failed {
                // original: BuildStatusUtils.IsErrorBuild (strictly Failed)
                self.fixed_projects.push(project.clone());
            } else {
                self.succeeding_projects.push(project.clone());
            }
        } else if status.treat_as_failure() {
            if last_completed.treat_as_failure() {
                self.still_failing_projects.push(project.clone());
            } else {
                self.failing_projects.push(project.clone());
            }
        }
    }
}

/// Notification service that plays the transition sound after every update
/// (original: `NotificationService`).
pub struct NotificationService {
    tray: Option<Box<dyn TrayNotifier>>,
    /// The buckets live across calls (original: readonly field).
    all_servers_status: Mutex<AllServersStatus>,
}

impl NotificationService {
    /// Create a new notification service.
    pub fn new() -> Self {
        Self {
            tray: None,
            all_servers_status: Mutex::new(AllServersStatus::new()),
        }
    }

    /// Set the tray notifier implementation.
    pub fn set_tray_notifier(&mut self, tray: Box<dyn TrayNotifier>) {
        self.tray = Some(tray);
    }

    /// Recompute the buckets and play the sound of the most severe
    /// transition (original: NotificationService.Execute, subscribed to
    /// `ConfigurationUpdated` and `ProjectsUpdated`).
    ///
    /// The sound priority is still failing > failing > fixed > succeeding,
    /// exactly like the original.
    pub fn execute(&self, config: &Configuration) -> Result<()> {
        let mut buckets = match self.all_servers_status.lock() {
            Ok(buckets) => buckets,
            Err(poisoned) => poisoned.into_inner(),
        };
        buckets.update(&config.servers);

        if !config.notification_settings.sound_notifications {
            return Ok(());
        }

        let sound_key = if !buckets.still_failing_projects.is_empty() {
            "still_failing"
        } else if !buckets.failing_projects.is_empty() {
            "failed"
        } else if !buckets.fixed_projects.is_empty() {
            "fixed"
        } else if !buckets.succeeding_projects.is_empty() {
            "succeeded"
        } else {
            return Ok(());
        };

        let path = config
            .notification_settings
            .sound_paths
            .as_ref()
            .and_then(|paths| paths.get(sound_key))
            .cloned();
        if let Some(path) = path {
            self.play_file(&path);
        }
        Ok(())
    }

    /// Play a sound file through the tray notifier
    /// (original: SoundPlayer.PlayFile, also used by the settings "Test"
    /// buttons).
    pub fn play_sound(&self, path: &str) -> Result<()> {
        self.play_file(path);
        Ok(())
    }

    /// original: SoundPlayer.PlayFile - logs and skips missing files.
    fn play_file(&self, path: &str) {
        log::debug!("Playing sound: {path}");
        if path.is_empty() || !std::path::Path::new(path).is_file() {
            return;
        }
        match self.tray {
            Some(ref tray) => {
                if let Err(e) = tray.play_sound(path) {
                    log::error!("Could not play {path}: {e}");
                }
            }
            None => log::info!("Playing sound: {path}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AllBuildDetails, Project};

    fn server_with(project_status: BuildStatus) -> Server {
        let mut project = Project {
            name: "job".into(),
            url: "http://ci/job/job/".into(),
            ..Default::default()
        };
        project.all_build_details = Some(AllBuildDetails {
            status: project_status,
            ..Default::default()
        });
        Server {
            url: "http://ci/".into(),
            projects: vec![project],
            ..Default::default()
        }
    }

    fn status(value: BuildStatusEnum, in_progress: bool) -> BuildStatus {
        BuildStatus {
            value,
            is_in_progress: in_progress,
            is_stuck: false,
        }
    }

    use BuildStatusEnum::{Failed, Successful};

    /// The buckets only fire when the *previous* status was in progress and a
    /// completed status is on record - the original needs this history to tell
    /// "failed" from "still failing" and "fixed" from "succeeding".
    #[test]
    fn buckets_track_transitions() {
        let mut all = AllServersStatus::new();

        // A successful build runs and completes as failed -> failing.
        all.update(&[server_with(status(Successful, false))]);
        all.update(&[server_with(status(Successful, true))]);
        all.update(&[server_with(status(Failed, false))]);
        assert_eq!(all.failing_projects.len(), 1);
        assert!(all.still_failing_projects.is_empty());

        // Another failing build runs and completes -> still failing.
        all.update(&[server_with(status(Failed, true))]);
        all.update(&[server_with(status(Failed, false))]);
        assert_eq!(all.still_failing_projects.len(), 1);
        assert!(all.failing_projects.is_empty());

        // A successful build runs and completes -> fixed.
        all.update(&[server_with(status(Successful, true))]);
        all.update(&[server_with(status(Successful, false))]);
        assert_eq!(all.fixed_projects.len(), 1);
        assert!(all.succeeding_projects.is_empty());

        // The next successful build completes -> succeeding.
        all.update(&[server_with(status(Successful, true))]);
        all.update(&[server_with(status(Successful, false))]);
        assert_eq!(all.succeeding_projects.len(), 1);
        assert!(all.fixed_projects.is_empty());
    }

    #[test]
    fn no_buckets_while_only_completed_states_change() {
        let mut all = AllServersStatus::new();
        all.update(&[server_with(status(Successful, false))]);
        all.update(&[server_with(status(Successful, false))]);
        assert!(all.succeeding_projects.is_empty());
        assert!(all.fixed_projects.is_empty());
    }
}
