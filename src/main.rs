//! Main entry point for the Jenkins Tray application.
//!
//! Port of the original C# application wiring (`Program.cs`, `MainForm`,
//! `SettingsForm`, `ServerListControl`/`ProjectListControl`, `EditServerForm`,
//! `AboutForm`, `AuthenticationTokenForm` and `ClaimBuildForm`): composes the
//! services, fills the Slint windows and forwards every UI callback to the
//! corresponding service call, mirroring the original event handlers.
//!
//! The Jenkins client, the update service, the notification buckets and the
//! tray notifier state are ports of `JenkinsService`, `ProjectsUpdateService`,
//! `AllServersStatus`/`NotificationService` and `TrayNotifier`; where the Rust
//! backend is still a stub (sound playback), the callback degrades to logging
//! - the original behavior is documented at each call site.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use chrono::{Duration, Local, TimeZone};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};

use app_update::{ApplicationUpdateService, UpdateSource};
use config::ConfigurationService;
use model::{BuildCauseEnum, BuildDetails, BuildStatusEnum, Configuration, Project, Server};
use notifications::NotificationService;
use projects_updater::{ProjectsUpdateService, UpdaterConfig};
use tray::TrayHub;

slint::include_modules!();

mod app_update;
mod config;
mod errors;
mod jenkins_client;
mod model;
mod notifications;
mod projects_updater;
mod tray;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// What the Edit Server dialog is editing (`index` is `None` when adding).
#[derive(Default, Clone)]
struct EditServerContext {
    index: Option<usize>,
}

/// Sortable column of the main grid (original: the grid columns with an
/// explicit `SortMode`: name and details sort by display text, the three
/// build date columns use the custom date comparer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Details,
    LastBuild,
    LastSuccess,
    LastFailure,
}

/// The active column sort. `None` reproduces the original's initial grid
/// state (`SortInfo`: server ascending, then project name ascending); the
/// column headers cycle ascending -> descending -> unsorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridSort {
    key: SortKey,
    ascending: bool,
}

/// The sort state after clicking the header of `key`'s column: first click
/// sorts ascending, the second descending, the third removes the sort
/// (original: DevExpress column header click cycle).
fn next_sort(current: Option<GridSort>, key: SortKey) -> Option<GridSort> {
    match current {
        Some(active) if active.key == key && active.ascending => {
            Some(GridSort { key, ascending: false })
        }
        Some(active) if active.key == key => None,
        _ => Some(GridSort { key, ascending: true }),
    }
}

/// State shared between the Slint callbacks.
#[derive(Default)]
struct AppState {
    /// Mirrors the UI `rows` property: `None` for server group headers,
    /// `Some((server_url, project_url))` for project rows. Used to resolve the
    /// selected row when a context menu action fires.
    row_keys: Vec<Option<(String, String)>>,
    /// Active grid sort (original: the DevExpress column SortInfo).
    sort: Option<GridSort>,
    /// Projects available to the settings checkbox list, keyed by server URL.
    /// Filled with a live Jenkins job list (original:
    /// `ProjectListControl.UpdateProjectList` -> `JenkinsService.LoadProjects`)
    /// and seeded with the configured projects until the first load.
    settings_projects: HashMap<String, Vec<Project>>,
    /// Server currently being edited in the Edit Server dialog.
    edit_server: EditServerContext,
    /// Project targeted by the authentication token dialog.
    auth_token_target: Option<(String, String)>,
    /// Project targeted by the claim build dialog:
    /// `(server_url, project_url, claimed build number)`.
    claim_target: Option<(String, String, u32)>,
    /// Project targeted by the "Build with parameters" dialog:
    /// `(server_url, project_url)` (new feature).
    build_params_target: Option<(String, String)>,
    /// The `(name, value)` pairs shown in the "Build with parameters"
    /// dialog: initialized from the job's defaults when the dialog opens
    /// and updated by its `param-edited` callback (new feature).
    build_params: Vec<(String, String)>,
}

/// Services captured by the Slint callbacks.
#[derive(Clone)]
struct Services {
    config: Arc<ConfigurationService>,
    notifications: Arc<NotificationService>,
    /// Pooled HTTP clients (strict / ignore-untrusted-certificate).
    clients: jenkins_client::Clients,
    updater: Rc<RefCell<ProjectsUpdateService>>,
    state: Arc<Mutex<AppState>>,
    /// Tray backend + notifier state (original: `TrayNotifier.Instance`).
    tray: Arc<Mutex<TrayHub>>,
    /// Application update service (original: `ApplicationUpdateService`).
    app_update: Arc<ApplicationUpdateService>,
    /// Handle used to spawn the Jenkins HTTP calls on the tokio runtime.
    runtime: tokio::runtime::Handle,
}

/// Locks the shared UI state, ignoring poisoning (the previous `RefCell`
/// borrow would panic in the same situation).
fn lock_state(state: &Arc<Mutex<AppState>>) -> MutexGuard<'_, AppState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The dialog windows. Like the original forms, each one is created once and
/// shown/hidden on demand.
#[derive(Clone)]
struct Windows {
    settings: Rc<SettingsWindow>,
    edit_server: Rc<EditServerWindow>,
    about: Rc<AboutWindow>,
    auth_token: Rc<AuthTokenWindow>,
    claim_build: Rc<ClaimBuildWindow>,
    build_params: Rc<BuildParamsWindow>,
    message_box: Rc<MessageBoxWindow>,
}

// ---------------------------------------------------------------------------
// Formatting helpers (ported from MainForm's ProjectWrapper)
// ---------------------------------------------------------------------------

/// Server caption (original: Server.DisplayText).
fn server_display_text(server: &Server) -> String {
    if server.display_name.is_empty() {
        server.url.clone()
    } else {
        server.display_name.clone()
    }
}

/// Looks up a project behind a grid row.
fn project_in<'a>(
    config: &'a Configuration,
    server_url: &str,
    project_url: &str,
) -> Option<(&'a Server, &'a Project)> {
    let server = config.servers.iter().find(|s| s.url == server_url)?;
    let project = server.projects.iter().find(|p| p.url == project_url)?;
    Some((server, project))
}

/// Server/project URLs of the selected grid row (`None` for headers).
fn selected_target(
    ui: &AppWindow,
    state: &Arc<Mutex<AppState>>,
) -> Option<(String, String)> {
    let index = ui.get_selected_row() as usize;
    lock_state(state).row_keys.get(index).cloned().flatten()
}

/// User names sorted alphabetically (the original joins them with ", ").
fn sorted_names(users: &HashSet<String>) -> Vec<String> {
    let mut names: Vec<String> = users.iter().cloned().collect();
    names.sort();
    names
}

/// Formats a user column (original FormatUsers: "-" without a build, otherwise
/// the users joined with ", ").
fn format_users(users: Option<&HashSet<String>>) -> String {
    let Some(users) = users else {
        return "-".to_string();
    };
    sorted_names(users).join(", ")
}

/// Formats one "Last build / Last success / Last failure" cell
/// (original ProjectWrapper.FormatBuildDetails: "Today 2:32 PM (#123)").
fn format_build(number: u32, time: Option<i64>) -> String {
    if number == 0 {
        return "-".to_string();
    }
    let Some(timestamp) = time else {
        return format!("(#{number})");
    };
    // Jenkins reports timestamps in milliseconds; tolerate seconds as well.
    let millis = if timestamp.abs() < 100_000_000_000 {
        timestamp * 1000
    } else {
        timestamp
    };
    let Some(build_time) = Local.timestamp_millis_opt(millis).single() else {
        return format!("(#{number})");
    };

    let today = Local::now().date_naive();
    let day = build_time.date_naive();
    let date = if day == today {
        "Today".to_string()
    } else if day == today - Duration::days(1) {
        "Yesterday".to_string()
    } else {
        // original: DateTime.ToString("d") - short date pattern
        build_time.format("%m/%d/%Y").to_string()
    };
    // original: DateTime.ToString("t") - short time pattern
    format!(
        "{} {} (#{number})",
        date,
        build_time.format("%-I:%M %p")
    )
}

/// Uppercases the first letter (the original formats the status enum name,
/// e.g. "Failed. ").
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Builds the "Details" column (original:
/// ProjectWrapper.FormatBuildDetailsAndSummary).
fn format_details(project: &Project) -> String {
    let Some(details) = project.all_build_details.as_ref() else {
        return String::new();
    };
    let Some(last_build) = details.last_build.as_ref() else {
        return String::new();
    };
    let Some(last_build_causes) = last_build.causes.as_ref() else {
        return String::new();
    };

    let status = project.status();
    let mut details_text = String::new();

    if status.is_in_progress {
        // original: time left + causes summary
        let time_left = format_estimated_duration(last_build);
        let mut causes_summary = String::new();
        for cause in &last_build_causes.causes {
            if last_build_causes.has_unique_causes == Some(false) {
                causes_summary = "Started by multiple sources. ".to_string();
                break;
            }
            match cause.cause {
                BuildCauseEnum::SCM => {
                    if last_build.users.is_empty() {
                        causes_summary = "Started by SCM change. ".to_string();
                    } else if last_build.users.len() > 1 {
                        causes_summary =
                            format!("Changes by {} persons. ", last_build.users.len());
                    } else {
                        causes_summary =
                            format!("Changes by {}. ", format_users(Some(&last_build.users)));
                    }
                }
                BuildCauseEnum::User => {
                    causes_summary =
                        format!("Started by {}. ", cause.starter.clone().unwrap_or_default());
                }
                BuildCauseEnum::RemoteHost => causes_summary = "Started remotely. ".to_string(),
                BuildCauseEnum::Timer => causes_summary = "Scheduled. ".to_string(),
                BuildCauseEnum::UpstreamProject => {
                    causes_summary = "Started by upstream project. ".to_string();
                }
                BuildCauseEnum::Unknown => {}
            }
        }
        details_text = format!("{time_left}{causes_summary}");
    } else {
        match status.value {
            BuildStatusEnum::Successful => details_text.push_str(&format_duration(last_build)),
            BuildStatusEnum::Unstable => details_text.push_str("Unstable."),
            BuildStatusEnum::Disabled => details_text.push_str("Project disabled. "),
            value => {
                details_text.push_str(&capitalize(&value.to_string()));
                details_text.push_str(". ");
                if !last_build.users.is_empty() {
                    details_text
                        .push_str(&format!("Broken by {}. ", format_users(Some(&last_build.users))));
                }
            }
        }

        // original: BuildDetails_InQueue_Why = "Reason: {0}."
        if let Some(why) = project
            .queue
            .as_ref()
            .filter(|queue| queue.in_queue)
            .and_then(|queue| queue.why.as_deref())
            .filter(|why| !why.is_empty())
        {
            details_text.push_str(&format!("Reason: {why}."));
        }
    }

    if status.is_stuck {
        let why = project
            .queue
            .as_ref()
            .and_then(|queue| queue.why.as_deref())
            .unwrap_or("");
        details_text = if why.is_empty() {
            "Queued, possibly stuck. ".to_string()
        } else {
            format!("Queued, possibly stuck. Reason: {why}.")
        };
    }

    details_text
}

/// "Took X minutes. " / "Took X hour(s) Y minutes. "
/// (original: ProjectWrapper.FormatDuration).
fn format_duration(build: &BuildDetails) -> String {
    let Some(duration_ms) = build.duration else {
        return String::new();
    };
    let duration = Duration::milliseconds(duration_ms);
    if duration.num_hours() > 1 {
        format!(
            "Took {} hour(s) {} minutes. ",
            duration.num_days() * 24 + duration.num_hours(),
            (duration.num_minutes() % 60).abs()
        )
    } else if duration.num_minutes() < 1 {
        "Took less than a minute. ".to_string()
    } else {
        let minutes = (duration.num_minutes() % 60).abs();
        let rounded = minutes + i64::from(duration.num_seconds() % 60 >= 30);
        format!("Took {} minutes. ", rounded.max(1))
    }
}

/// Time left of an in-progress build (original:
/// ProjectWrapper.FormatEstimatedDuration).
fn format_estimated_duration(build: &BuildDetails) -> String {
    let Some(estimated_ms) = build.estimated_duration else {
        return String::new();
    };
    // Jenkins timestamps are milliseconds; tolerate seconds as well.
    let millis = if build.time.abs() < 100_000_000_000 {
        build.time * 1000
    } else {
        build.time
    };
    let Some(build_time) = Local.timestamp_millis_opt(millis).single() else {
        return String::new();
    };
    // original: endtime = Time.Add(EstimatedDuration); timeleft = endtime - UtcNow
    let time_left = build_time + Duration::milliseconds(estimated_ms) - Local::now();

    let total_hours = time_left.num_minutes() as f64 / 60.0;
    let hours = time_left.num_hours().abs();
    let minutes = (time_left.num_minutes() % 60).abs();
    if total_hours >= 1.0 {
        format!("{hours} hour(s) {minutes} minutes remaining. ")
    } else if total_hours < -1.0 {
        format!("{hours} hours {minutes} minutes longer than usual. ")
    } else if total_hours < 0.0 {
        format!("{minutes} minutes longer than usual. ")
    } else {
        format!("{minutes} minutes remaining. ")
    }
}

/// Formats one build for the "Last build / Last success / Last failure"
/// columns (original: ProjectWrapper.FormatBuildDetails; "-" without a build).
fn format_build_details(build: Option<&BuildDetails>) -> String {
    let Some(build) = build else {
        return "-".to_string();
    };
    format_build(build.number, Some(build.time))
}

/// The "Project" cell text (original: `ProjectWrapper.Name` - the display
/// name is used when the setting is enabled, falling back to the project name
/// when it is empty).
fn display_name(project: &Project, show_display_name: bool) -> String {
    let display_name = project
        .display_name
        .as_deref()
        .filter(|name| !name.is_empty());
    match (show_display_name, display_name) {
        (true, Some(display)) => display.to_string(),
        _ => project.name.clone(),
    }
}

/// Row hover tooltip (original: `MainForm.projectsGridView_MouseMove`): the
/// `BuildStatus_*` resource for the project's status plus the "in queue since"
/// suffix while a stuck build is queued. Empty means no tooltip (the original
/// passes a null resource string to `SetToolTip`).
fn status_tooltip(project: &Project) -> String {
    let status = project.status();
    let mut message = status_resource(&status.resource_key())
        .unwrap_or_default()
        .to_string();

    // original: if (Status.IsStuck && Queue.InQueue) message +=
    // string.Format(BuildDetails_InQueue_Since, Queue.InQueueSince)
    if status.is_stuck {
        let queued_since = project
            .queue
            .as_ref()
            .filter(|queue| queue.in_queue)
            .and_then(|queue| queue.in_queue_since);
        if let Some(since) = queued_since.and_then(|millis| Local.timestamp_millis_opt(millis).single())
        {
            // original: DateTime default representation ("10/12/2026 11:47:41 AM")
            message.push_str(&format!(
                " In queue since {}.",
                since.format("%m/%d/%Y %-I:%M:%S %p")
            ));
        }
    }
    message
}

/// The `BuildStatus_*` strings from `JenkinsTrayResources.resx`; `None` when
/// the original resource manager has no entry for the key (e.g. a combined
/// in-progress + stuck state), in which case the original shows no tooltip.
fn status_resource(key: &str) -> Option<&'static str> {
    Some(match key {
        "BuildStatus_Aborted" => "Build was aborted",
        "BuildStatus_Aborted_Stuck" => "Build was aborted. Build is stuck.",
        "BuildStatus_Disabled" => "Project disabled",
        "BuildStatus_Disabled_BuildInProgress" => "Project disabled. Build in progress.",
        "BuildStatus_Disabled_Stuck" => "Project disabled. Build is stuck.",
        "BuildStatus_Failed" => "Build failed",
        "BuildStatus_Failed_BuildInProgress" => "Last build failed. Build in progress.",
        "BuildStatus_Failed_Stuck" => "Last build failed. Build is stuck.",
        "BuildStatus_Indeterminate" => "Indeterminate: no build done.",
        "BuildStatus_Indeterminate_BuildInProgress" => {
            "Indeterminate: no build done. Build in progress."
        }
        "BuildStatus_Indeterminate_Stuck" => "Indeterminate: no build done. Build is stuck.",
        "BuildStatus_Successful" => "Successful",
        "BuildStatus_Successful_BuildInProgress" => "Successful. Build in progress.",
        "BuildStatus_Successful_Stuck" => "Successful. Build is stuck.",
        "BuildStatus_Unknown" => "Unknown status",
        "BuildStatus_Unstable" => "Unstable: some tests failed.",
        "BuildStatus_Unstable_BuildInProgress" => "Unstable: some tests failed. Build in progress.",
        "BuildStatus_Unstable_Stuck" => "Unstable: some tests failed. Build is stuck.",
        _ => return None,
    })
}

/// Builds one grid row for a project (original: ProjectWrapper properties).
fn project_row(project: &Project, server: &Server, show_display_name: bool) -> ProjectRow {
    let details = project.all_build_details.as_ref();
    let status = project.status();
    let value = status.value;
    let in_progress = status.is_in_progress;
    let stuck = status.is_stuck;
    let queued = project
        .queue
        .as_ref()
        .map(|queue| queue.in_queue)
        .unwrap_or(false);

    let name = display_name(project, show_display_name);

    let last_build = format_build_details(details.and_then(|d| d.last_build.as_ref()));
    let last_success_build = details.and_then(|d| d.last_successful_build.as_ref());
    let last_failure_build = details.and_then(|d| d.last_failed_build.as_ref());
    let last_success = format_build_details(last_success_build);
    let last_success_user = format_users(last_success_build.map(|build| &build.users));
    let last_failure = format_build_details(last_failure_build);
    let last_failure_user = format_users(last_failure_build.map(|build| &build.users));
    // original: ClaimedBy/ClaimReason come from the last failed build's claim
    // details (ClaimDetails.Assignee / .Reason).
    let (claimed_by, claim_reason) = last_failure_build
        .and_then(|build| build.claim_details.as_ref())
        .map(|claim| (claim.assignee.clone(), claim.reason.clone()))
        .unwrap_or_default();

    ProjectRow {
        server: server_display_text(server).into(),
        name: name.into(),
        url: project.url.clone().into(),
        details: format_details(project).into(),
        last_build: last_build.into(),
        last_success: last_success.into(),
        last_success_user: last_success_user.into(),
        last_failure: last_failure.into(),
        last_failure_user: last_failure_user.into(),
        claimed_by: claimed_by.into(),
        claim_reason: claim_reason.into(),
        status_kind: value.to_string().into(),
        in_progress,
        stuck,
        queued,
        status_tooltip: status_tooltip(project).into(),
        // original: projectStatus.IsStuck || Value >= BuildStatusEnum.Indeterminate
        ack_status_eligible: stuck || value >= BuildStatusEnum::Indeterminate,
        // original: project.LastFailedBuild != null
        claim_eligible: last_failure_build.is_some(),
        is_header: false,
    }
}

/// Builds the main grid: one group header per server followed by its projects
/// (original: `projectsGridControl` rows).
///
/// Servers are always ordered by display text and projects by the active
/// column sort; with no sort (`sort == None`) the original's initial
/// `SortInfo` applies (server ascending, then project name ascending).
/// Documented deviation: the original sorts a flat row list, so sorting by a
/// project column dissolves the server grouping - the port keeps the group
/// headers and sorts the projects within each group instead.
fn build_rows(
    config: &Configuration,
    sort: Option<GridSort>,
) -> (Vec<ProjectRow>, Vec<Option<(String, String)>>) {
    let show_display_name = config.general_settings.show_project_display_name;
    let mut rows = Vec::new();
    let mut keys: Vec<Option<(String, String)>> = Vec::new();

    // original: initial SortInfo = serverGridColumn ascending
    let mut servers: Vec<&Server> = config.servers.iter().collect();
    servers.sort_by(|a, b| {
        server_display_text(a)
            .to_lowercase()
            .cmp(&server_display_text(b).to_lowercase())
    });

    for server in servers {
        rows.push(ProjectRow {
            server: server_display_text(server).into(),
            is_header: true,
            ..Default::default()
        });
        keys.push(None);

        let mut projects: Vec<&Project> = server.projects.iter().collect();
        match sort {
            // original: nameGridColumn ascending (sorted by display text,
            // matching ColumnSortMode.DisplayText)
            None => projects.sort_by(|a, b| {
                display_name(a, show_display_name)
                    .to_lowercase()
                    .cmp(&display_name(b, show_display_name).to_lowercase())
            }),
            Some(sort) => {
                projects.sort_by(|a, b| {
                    let order = compare_projects(a, b, sort.key, show_display_name);
                    if sort.ascending { order } else { order.reverse() }
                });
            }
        }

        for project in projects {
            rows.push(project_row(project, server, show_display_name));
            keys.push(Some((server.url.clone(), project.url.clone())));
        }
    }

    (rows, keys)
}

/// Compares two projects by a sortable column (original: the grid's
/// `SortMode` handlers - DisplayText comparison for name/details, the build
/// date for the three date columns with null dates first, see
/// `MainForm.DoCustomColumnSort`).
fn compare_projects(
    a: &Project,
    b: &Project,
    key: SortKey,
    show_display_name: bool,
) -> std::cmp::Ordering {
    let time_of = |build: Option<&BuildDetails>| build.map(|build| build.time);
    match key {
        SortKey::Name => display_name(a, show_display_name)
            .to_lowercase()
            .cmp(&display_name(b, show_display_name).to_lowercase()),
        SortKey::Details => format_details(a).to_lowercase().cmp(&format_details(b).to_lowercase()),
        SortKey::LastBuild => time_of(
            a.all_build_details.as_ref().and_then(|d| d.last_build.as_ref()),
        )
        .cmp(&time_of(
            b.all_build_details.as_ref().and_then(|d| d.last_build.as_ref()),
        )),
        SortKey::LastSuccess => time_of(
            a.all_build_details
                .as_ref()
                .and_then(|d| d.last_successful_build.as_ref()),
        )
        .cmp(&time_of(
            b.all_build_details
                .as_ref()
                .and_then(|d| d.last_successful_build.as_ref()),
        )),
        SortKey::LastFailure => time_of(
            a.all_build_details
                .as_ref()
                .and_then(|d| d.last_failed_build.as_ref()),
        )
        .cmp(&time_of(
            b.all_build_details
                .as_ref()
                .and_then(|d| d.last_failed_build.as_ref()),
        )),
    }
}

// ---------------------------------------------------------------------------
// Window helpers
// ---------------------------------------------------------------------------

/// Rebuilds the main grid from the configuration and refreshes the status bar
/// (original: MainForm.LoadProjects + UpdateStatusBar).
fn refresh_rows(
    ui: &AppWindow,
    config_service: &ConfigurationService,
    state: &Arc<Mutex<AppState>>,
) -> Result<()> {
    let config = config_service.get_config()?;
    let sort = lock_state(state).sort;
    let (rows, keys) = build_rows(&config, sort);
    lock_state(state).row_keys = keys;
    ui.set_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_integrate_claim(config.general_settings.integrate_with_claim_plugin);
    // original: string.Format(LastCheck_Format, DateTime.Now)
    ui.set_last_check(
        format!(
            "Last check: {}",
            Local::now().format("%m/%d/%Y %-I:%M:%S %p")
        )
        .into(),
    );
    Ok(())
}

/// The original `ConfigurationUpdated` event: refresh the grid and run the
/// notification service + tray notifier (subscribed by `MainForm`,
/// `TrayNotifier` and `NotificationService` whenever the configuration is
/// saved).
fn on_configuration_updated(services: &Services, ui: &AppWindow) {
    if let Err(e) = refresh_rows(ui, &services.config, &services.state) {
        log::error!("Could not refresh the projects grid: {e}");
    }
    if let Ok(config) = services.config.get_config() {
        if let Err(e) = services.notifications.execute(&config) {
            log::error!("Could not run the notification service: {e}");
        }
        if let Ok(mut tray) = services.tray.lock() {
            tray.update(&config);
        }
    }
}

/// Fills the settings dialog from the configuration and starts loading the
/// live job list of the selected server
/// (original: SettingsForm initialization on show -> UpdateProjectList).
fn populate_settings(services: &Services, settings: &SettingsWindow) -> Result<()> {
    let config = services.config.get_config()?;

    let servers: Vec<ServerRow> = config
        .servers
        .iter()
        .map(|server| ServerRow {
            display: server_display_text(server).into(),
            url: server.url.clone().into(),
        })
        .collect();
    settings.set_servers(ModelRc::new(VecModel::from(servers)));

    let general = &config.general_settings;
    settings.set_refresh_interval(general.refresh_interval_in_seconds as i32);
    settings.set_update_main_window_icon(general.update_main_window_icon);
    settings.set_integrate_with_claim_plugin(general.integrate_with_claim_plugin);
    settings.set_show_project_display_name(general.show_project_display_name);
    settings.set_check_for_updates(general.check_for_updates);

    let notifications = &config.notification_settings;
    settings.set_enable_sound(notifications.sound_notifications);
    settings.set_treat_unstable_as_failed(notifications.treat_unstable_as_failed);
    settings.set_enable_balloon(notifications.balloon_notifications);
    let sound_paths = notifications.sound_paths.clone().unwrap_or_default();
    settings.set_sound_failed(sound_paths.get("failed").cloned().unwrap_or_default().into());
    settings.set_sound_fixed(sound_paths.get("fixed").cloned().unwrap_or_default().into());
    settings.set_sound_still_failing(
        sound_paths
            .get("still_failing")
            .cloned()
            .unwrap_or_default()
            .into(),
    );
    settings.set_sound_succeeded(
        sound_paths
            .get("succeeded")
            .cloned()
            .unwrap_or_default()
            .into(),
    );

    // original: Controller.SetStatus("Loading projects from {url}", true)
    // while the live job list loads in the background.
    settings.set_servers_status("".into());

    let count = config.servers.len() as i32;
    let selected = settings.get_selected_server();
    let selected = if count > 0 && (0..count).contains(&selected) {
        selected
    } else if count > 0 {
        0
    } else {
        -1
    };
    settings.set_selected_server(selected);
    populate_project_list(&services.config, settings, &services.state, selected)?;
    if let Some(server) = config.servers.get(selected as usize) {
        spawn_project_load(services, settings, server);
    }
    Ok(())
}

/// Kicks off the live job list download for one server (original:
/// ProjectListControl.UpdateProjectList - `JenkinsService.LoadProjects` on a
/// background worker, re-enabling the window and setting the status
/// afterwards).
fn spawn_project_load(services: &Services, settings: &SettingsWindow, server: &Server) {
    let settings_weak = settings.as_weak();
    // original: string.Format(LoadingProjects_FormatString, server.Url)
    settings_weak
        .upgrade()
        .map(|settings| settings.set_servers_status(format!("Loading projects from {}", server.url).into()));

    let client = services.clients.for_server(server).clone();
    let server = server.clone();
    let config_service = services.config.clone();
    let state = services.state.clone();
    services.runtime.spawn(async move {
        let result = jenkins_client::load_projects(&client, &server).await;
        let dispatched = slint::invoke_from_event_loop(move || {
            let Some(settings) = settings_weak.upgrade() else {
                return;
            };
            match result {
                Ok(projects) => {
                    // original: SetProjectsDataSource(projects) + status ""
                    lock_state(&state)
                        .settings_projects
                        .insert(server.url.clone(), projects);
                    settings.set_servers_status("".into());
                    let selected = settings.get_selected_server();
                    if let Err(e) =
                        populate_project_list(&config_service, &settings, &state, selected)
                    {
                        log::error!("Could not refresh the project list: {e}");
                    }
                }
                Err(e) => {
                    log::error!(
                        "Error loading projects from {}: {}",
                        server.url,
                        errors::root_cause(&e)
                    );
                    settings.set_servers_status(
                        format!("Error loading projects from {}", server.url).into(),
                    );
                }
            }
        });
        if let Err(e) = dispatched {
            log::error!("Could not apply the loaded projects: {e}");
        }
    });
}

/// Fills the settings checkbox list for one server (original:
/// ProjectListControl.SetProjectsDataSource; the list shows the live job list
/// once it arrived, and falls back to the configured projects).
fn populate_project_list(
    config_service: &ConfigurationService,
    settings: &SettingsWindow,
    state: &Arc<Mutex<AppState>>,
    server_index: i32,
) -> Result<()> {
    let config = config_service.get_config()?;
    let Some(server) = config.servers.get(server_index as usize) else {
        settings.set_projects(ModelRc::new(VecModel::from(Vec::<ProjectCheckRow>::new())));
        return Ok(());
    };

    let rows: Vec<ProjectCheckRow> = {
        let mut state = lock_state(state);
        let available = state
            .settings_projects
            .entry(server.url.clone())
            .or_insert_with(|| server.projects.clone());
        available
            .iter()
            .map(|project| ProjectCheckRow {
                name: project.name.clone().into(),
                selected: server.projects.iter().any(|p| p.url == project.url),
            })
            .collect()
    };
    settings.set_projects(ModelRc::new(VecModel::from(rows)));
    Ok(())
}

/// Stores the sound paths of the notification settings, skipping empty ones.
fn set_sound_path(paths: &mut HashMap<String, String>, key: &str, value: String) {
    if value.trim().is_empty() {
        paths.remove(key);
    } else {
        paths.insert(key.to_string(), value);
    }
}

/// URL validation from EditServerForm.IsUrlValid.
fn is_valid_server_url(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty() && url != "http://" && url != "https://"
}

/// URL normalization from EditServerForm.urlTextBox_Leave: add the scheme when
/// missing and always end with a slash.
fn normalize_server_url(url: &str) -> String {
    let mut url = url.trim().to_string();
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("http://{url}");
    }
    if !url.ends_with('/') {
        url.push('/');
    }
    url
}

/// Console URL of the last build (original: JenkinsService.GetConsolePage).
fn console_page(project: &Project) -> String {
    if project.has_build() {
        format!("{}lastBuild/console", project.url)
    } else {
        project.url.clone()
    }
}

/// Opens a URL with the platform browser (original: UIUtils.OpenWebPage).
fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/c", "start", "", url]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };

    match command.spawn() {
        Ok(_) => log::info!("Opening {url}"),
        Err(e) => log::error!("Could not open {url}: {e}"),
    }
}

/// Action to run when the message box's "Yes" button is pressed (original:
/// `ApplicationUpdateHandler`'s Yes branch: open the installer page and exit).
/// OK-only boxes never register one.
static MESSAGE_BOX_YES_ACTION: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// Takes (and clears) the pending "Yes" action, if any.
fn take_message_box_yes_action() -> Option<Box<dyn Fn() + Send>> {
    MESSAGE_BOX_YES_ACTION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// Shows the generic message box, callable from any thread
/// (original: `XtraMessageBox.Show(message, caption, MessageBoxButtons.OK)`
/// and `MessageBox.Show(message, caption, OK, Error)`).
///
/// Documented deviation: the original dialogs are modal, this one is not.
///
/// Clears any stale "Yes" action: an OK box can never run the pending action
/// of an earlier Yes/No box.
fn show_message_box(message_box: &slint::Weak<MessageBoxWindow>, caption: &str, message: &str) {
    drop(take_message_box_yes_action());
    let message_box = message_box.clone();
    let caption = caption.to_string();
    let message = message.to_string();
    if let Err(e) = slint::invoke_from_event_loop(move || {
        let Some(message_box) = message_box.upgrade() else {
            return;
        };
        message_box.set_caption(caption.into());
        message_box.set_message(message.into());
        message_box.set_yes_no(false);
        if let Err(e) = message_box.show() {
            log::error!("Could not show the message box: {e}");
        }
    }) {
        log::error!("Could not schedule the message box: {e}");
    }
}

/// Shows a Yes/No message box; `on_yes` runs when the user presses "Yes"
/// (original: XtraMessageBox with MessageBoxButtons.YesNo - used by the
/// `ApplicationUpdateHandler` prompt).
fn show_yes_no_message_box(
    message_box: &slint::Weak<MessageBoxWindow>,
    caption: &str,
    message: &str,
    on_yes: Box<dyn Fn() + Send>,
) {
    *MESSAGE_BOX_YES_ACTION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(on_yes);
    let message_box = message_box.clone();
    let caption = caption.to_string();
    let message = message.to_string();
    if let Err(e) = slint::invoke_from_event_loop(move || {
        let Some(message_box) = message_box.upgrade() else {
            return;
        };
        message_box.set_caption(caption.into());
        message_box.set_message(message.into());
        message_box.set_yes_no(true);
        if let Err(e) = message_box.show() {
            log::error!("Could not show the message box: {e}");
        }
    }) {
        log::error!("Could not schedule the message box: {e}");
    }
}

// ---------------------------------------------------------------------------
// Callback wiring
// ---------------------------------------------------------------------------

/// Registers every Slint callback, mirroring the original form event handlers.
fn wire_callbacks(ui: &AppWindow, services: &Services, windows: &Windows) {
    // --- Main window -------------------------------------------------------

    // Toolbar "Refresh" / "Refresh" button / F5
    // (original: MainForm_KeyUp -> ProjectsUpdateService.UpdateProjects()).
    ui.on_refresh_projects({
        let services = services.clone();
        let ui = ui.as_weak();
        move || {
            let ui = ui.unwrap();
            services.updater.borrow().refresh_all();
            ui.set_status_text("".into());
            if let Err(e) = refresh_rows(&ui, &services.config, &services.state) {
                log::error!("Could not refresh the projects grid: {e}");
            }
        }
    });

    // Toolbar "Settings" (original: SettingsForm.ShowDialog).
    ui.on_open_settings({
        let services = services.clone();
        let settings = windows.settings.clone();
        move || {
            if let Err(e) = populate_settings(&services, &settings) {
                log::error!("Could not load the settings dialog: {e}");
            }
            if let Err(e) = settings.show() {
                log::error!("Could not show the settings dialog: {e}");
            }
        }
    });

    // Toolbar "Check for updates" - the original runs
    // ApplicationUpdateService.CheckForUpdates_Synchronous(User): an update
    // raises the Yes/No download prompt, no update shows
    // ApplicationUpdates_NoUpdate_Text, errors show ErrorBoxCaption /
    // ErrorBoxMessage. Documented deviation: the port performs the request on
    // the tokio runtime (the original blocks the UI thread) and posts the
    // message boxes back to the UI thread.
    ui.on_check_for_updates({
        let services = services.clone();
        let message_box = windows.message_box.as_weak();
        move || {
            let app_update = services.app_update.clone();
            let message_box = message_box.clone();
            services.runtime.spawn(async move {
                match app_update.check(UpdateSource::User).await {
                    // An update was found: the NewVersionAvailable handler
                    // already showed the download prompt.
                    Ok(true) => {}
                    // original: XtraMessageBox.Show(ApplicationUpdates_NoUpdate_Text,
                    // ApplicationUpdates_Caption, OK, Information)
                    Ok(false) => show_message_box(
                        &message_box,
                        "Jenkins Tray updates",
                        "No update available.",
                    ),
                    // original: ErrorBoxCaption / ErrorBoxMessage with the
                    // exception message.
                    Err(e) => show_message_box(
                        &message_box,
                        "An error occurred",
                        &format!("The following error occurred:\n\nMessage: {e}"),
                    ),
                }
            });
        }
    });

    // Column header click - sort the grid (original: DevExpress column
    // header clicks cycle ascending -> descending -> unsorted for that
    // column).
    ui.on_sort_changed({
        let services = services.clone();
        let ui = ui.as_weak();
        move |column: i32| {
            let key = match column {
                1 => SortKey::Name,
                2 => SortKey::Details,
                3 => SortKey::LastBuild,
                4 => SortKey::LastSuccess,
                5 => SortKey::LastFailure,
                _ => return, // not a sortable column
            };
            let sort = {
                let mut state = lock_state(&services.state);
                let next = next_sort(state.sort, key);
                state.sort = next;
                next
            };
            if let Some(ui) = ui.upgrade() {
                ui.set_sort_column(sort.map_or(-1, |_| column));
                ui.set_sort_ascending(sort.map_or(true, |sort| sort.ascending));
                if let Err(e) = refresh_rows(&ui, &services.config, &services.state) {
                    log::error!("Could not refresh the projects grid: {e}");
                }
            }
        }
    });

    // Toolbar "About" (original: AboutForm.ShowDialog).
    ui.on_open_about({
        let about = windows.about.clone();
        move || {
            if let Err(e) = about.show() {
                log::error!("Could not show the About dialog: {e}");
            }
        }
    });

    // Toolbar "Exit" / tray "Exit" (original: Application.Exit).
    ui.on_exit_app(move || {
        if let Err(e) = slint::quit_event_loop() {
            log::error!("Could not quit the event loop: {e}");
        }
    });

    // Double-click on a row (original: projectsGridView_DoubleClick).
    ui.on_activate_project({
        let services = services.clone();
        let ui = ui.as_weak();
        move || {
            let ui = ui.unwrap();
            let Some((server_url, project_url)) = selected_target(&ui, &services.state) else {
                return;
            };
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(e) => {
                    log::error!("Could not read the configuration: {e}");
                    return;
                }
            };
            let Some((_, project)) = project_in(&config, &server_url, &project_url) else {
                return;
            };
            // original: BuildStatusUtils.IsErrorBuild(status) || status.IsInProgress
            let status = project.status();
            let open_console =
                status.value == BuildStatusEnum::Failed || status.is_in_progress;
            if open_console {
                open_url(&console_page(project));
            } else {
                open_url(&project.url);
            }
        }
    });

    // Selection change -> context menu checkmarks (original:
    // contextMenuStrip_Opening).
    ui.on_selection_changed({
        let services = services.clone();
        let ui = ui.as_weak();
        move |row_index| {
            let ui = ui.unwrap();
            let target = lock_state(&services.state)
                .row_keys
                .get(row_index as usize)
                .cloned()
                .flatten();
            let (server_url, project_url) = match target {
                Some(target) => target,
                None => {
                    ui.set_selected_ack_project(false);
                    ui.set_selected_ack_status(false);
                    return;
                }
            };
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(_) => return,
            };
            let Some((_, project)) = project_in(&config, &server_url, &project_url) else {
                return;
            };
            // original: acknowledgeProjectMenuItem.Checked = project.IsAcknowledged
            ui.set_selected_ack_project(project.is_acknowledged);
            // original: TrayNotifier.IsStatusAcknowledged(project.Url)
            let acknowledged = services
                .tray
                .lock()
                .map(|tray| tray.state.is_status_acknowledged(&project_url))
                .unwrap_or(false);
            ui.set_selected_ack_status(acknowledged);
        }
    });

    // Context menu actions.
    ui.on_menu_action({
        let services = services.clone();
        let ui = ui.as_weak();
        let auth_token = windows.auth_token.clone();
        let claim_build = windows.claim_build.clone();
        let build_params = windows.build_params.clone();
        let message_box = windows.message_box.as_weak();
        move |action| {
            let ui = ui.unwrap();
            let Some((server_url, project_url)) = selected_target(&ui, &services.state) else {
                log::warn!("Menu action '{action}' without a selected project");
                return;
            };
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(e) => {
                    log::error!("Could not read the configuration: {e}");
                    return;
                }
            };
            let Some((server, project)) = project_in(&config, &server_url, &project_url) else {
                return;
            };

            match action.as_str() {
                "open-page" => {
                    // original: UIUtils.OpenWebPage(project.Url)
                    open_url(&project.url);
                }
                "console" => {
                    // original: OpenProjectConsolePage
                    open_url(&console_page(project));
                }
                "run-build" => {
                    // original: JenkinsService.SafeRunBuild
                    let client = services.clients.for_server(server).clone();
                    let server = server.clone();
                    let project = project.clone();
                    let name = project.name.clone();
                    let message_box = message_box.clone();
                    services.runtime.spawn(async move {
                        if let Err(e) =
                            jenkins_client::trigger_build(&client, &server, &project).await
                        {
                            // original: LoggingHelper.LogError +
                            // XtraMessageBox.Show(RunBuildFailed_Text,
                            // RunBuildFailed_Caption)
                            let cause = errors::root_cause(&e);
                            log::error!("Failed to trigger a build of {name}: {cause}");
                            show_message_box(
                                &message_box,
                                "Error",
                                &format!("Failed running build: {cause}."),
                            );
                        } else {
                            log::info!("Build triggered for {name}");
                        }
                    });
                }
                "run-build-params" => {
                    // New feature (not in the original): the original's
                    // RunBuild posts `/build?delay=0sec`, which does not
                    // queue a run on parameterized jobs. Fetch the job's
                    // parameter definitions first and open the dialog that
                    // posts `buildWithParameters` with the values as form
                    // data, like the reference script
                    // `jenkins_files/trigger_pipeline.sh`.
                    lock_state(&services.state).build_params_target =
                        Some((server_url.clone(), project_url.clone()));
                    let client = services.clients.for_server(server).clone();
                    let server = server.clone();
                    let project = project.clone();
                    let job = project.name.clone();
                    let message_box = message_box.clone();
                    let dialog = build_params.as_weak();
                    let state = services.state.clone();
                    services.runtime.spawn(async move {
                        match jenkins_client::get_build_parameters(&client, &server, &project)
                            .await
                        {
                            Err(e) => {
                                let cause = errors::root_cause(&e);
                                log::error!(
                                    "Failed to load the build parameters of {job}: {cause}"
                                );
                                show_message_box(
                                    &message_box,
                                    "Error",
                                    &format!("Failed to load the build parameters: {cause}."),
                                );
                            }
                            Ok(params) if params.is_empty() => {
                                // A job without parameters behaves like the
                                // original "Run build".
                                log::info!(
                                    "{job} has no parameters - triggering a plain build"
                                );
                                if let Err(e) =
                                    jenkins_client::trigger_build(&client, &server, &project)
                                        .await
                                {
                                    let cause = errors::root_cause(&e);
                                    log::error!("Failed to trigger a build of {job}: {cause}");
                                    show_message_box(
                                        &message_box,
                                        "Error",
                                        &format!("Failed running build: {cause}."),
                                    );
                                } else {
                                    log::info!("Build triggered for {job}");
                                }
                            }
                            Ok(params) => {
                                // The dialog model and the authoritative
                                // name/value list (updated by `param-edited`)
                                // are both built on the event loop thread.
                                let _ = slint::invoke_from_event_loop(move || {
                                    let rows: Vec<BuildParamRow> = params
                                        .iter()
                                        .map(|param| BuildParamRow {
                                            name: param.name.as_str().into(),
                                            description: param.description.as_str().into(),
                                            kind: match param.kind {
                                                jenkins_client::ParamKind::Text => 0,
                                                jenkins_client::ParamKind::Choice => 1,
                                                jenkins_client::ParamKind::Boolean => 2,
                                                jenkins_client::ParamKind::Password => 3,
                                            },
                                            value: param.default.as_str().into(),
                                            choices: ModelRc::new(VecModel::from(
                                                param
                                                    .choices
                                                    .iter()
                                                    .map(|choice| choice.as_str().into())
                                                    .collect::<Vec<slint::SharedString>>(),
                                            )),
                                            choice_index: param
                                                .choices
                                                .iter()
                                                .position(|choice| *choice == param.default)
                                                .unwrap_or(0) as i32,
                                        })
                                        .collect();
                                    {
                                        let mut state = lock_state(&state);
                                        state.build_params = params
                                            .iter()
                                            .map(|param| {
                                                (param.name.clone(), param.default.clone())
                                            })
                                            .collect();
                                    }
                                    let dialog = dialog.unwrap();
                                    dialog.set_job_label(format!("Job: {job}").into());
                                    dialog.set_params(ModelRc::new(VecModel::from(rows)));
                                    if let Err(e) = dialog.show() {
                                        log::error!(
                                            "Could not show the build parameters dialog: {e}"
                                        );
                                    }
                                });
                            }
                        }
                    });
                }
                "abort-build" => {
                    let in_progress = project.status().is_in_progress;
                    if in_progress {
                        // original: JenkinsService.SafeStopBuild
                        let client = services.clients.for_server(server).clone();
                        let server = server.clone();
                        let project = project.clone();
                        let name = project.name.clone();
                        let message_box = message_box.clone();
                        services.runtime.spawn(async move {
                            if let Err(e) =
                                jenkins_client::stop_build(&client, &server, &project).await
                            {
                                // original: LoggingHelper.LogError +
                                // XtraMessageBox.Show(StopBuildFailed_Text,
                                // StopBuildFailed_Caption)
                                let cause = errors::root_cause(&e);
                                log::error!("Failed to stop the build of {name}: {cause}");
                                show_message_box(
                                    &message_box,
                                    "Error",
                                    &format!(
                                        "Failed stopping / cancelling queued build: {cause}."
                                    ),
                                );
                            } else {
                                log::info!("Build stop requested for {name}");
                            }
                        });
                    } else if let Some(queue_id) = project
                        .queue
                        .as_ref()
                        .and_then(|queue| queue.id)
                        .filter(|id| *id > 0)
                    {
                        // original: queue cancellation (Queue.Id > 0)
                        let client = services.clients.for_server(server).clone();
                        let server = server.clone();
                        let url = server.url.clone();
                        let message_box = message_box.clone();
                        services.runtime.spawn(async move {
                            if let Err(e) =
                                jenkins_client::remove_from_queue(&client, &server, queue_id).await
                            {
                                // original: StopBuildFailed box - the queue
                                // branch of stopBuildMenuItem_Click shares the
                                // same catch.
                                let cause = errors::root_cause(&e);
                                log::error!("Failed to cancel the queued build of {url}: {cause}");
                                show_message_box(
                                    &message_box,
                                    "Error",
                                    &format!(
                                        "Failed stopping / cancelling queued build: {cause}."
                                    ),
                                );
                            } else {
                                log::info!("Queue item {queue_id} cancelled ({url})");
                            }
                        });
                    }
                }
                "ack-status" => {
                    // original: acknowledgeMenuItem_Click - acknowledge only
                    // when the status is stuck or at least Indeterminate,
                    // otherwise clear.
                    // Note: Slint toggles `checked` *before* `activated` fires,
                    // so the property already holds the new state.
                    let now_checked = ui.get_selected_ack_status();
                    let status = project.status();
                    if now_checked {
                        if status.is_stuck || status.value >= BuildStatusEnum::Indeterminate {
                            // original: TrayNotifier.AcknowledgeStatus(project, status)
                            if let Ok(mut tray) = services.tray.lock() {
                                tray.state.acknowledge_status(&project_url, status.clone());
                            }
                            log::info!("Status acknowledged for {}", project.name);
                        }
                    } else {
                        // original: TrayNotifier.ClearAcknowledgedStatus(project)
                        if let Ok(mut tray) = services.tray.lock() {
                            tray.state.clear_acknowledged_status(&project_url);
                        }
                        log::info!("Status acknowledgement cleared for {}", project.name);
                    }
                    // original: AcknowledgeStatus / ClearAcknowledgedStatus
                    // trigger UpdateNotifier.
                    if let Ok(config) = services.config.get_config() {
                        if let Ok(mut tray) = services.tray.lock() {
                            tray.update(&config);
                        }
                    }
                }
                "ack-project" => {
                    // original: menuItem.Checked = project.IsAcknowledged = !Checked
                    let acknowledged = ui.get_selected_ack_project();
                    if let Err(e) =
                        services
                            .config
                            .set_project_acknowledged(&server_url, &project_url, acknowledged)
                    {
                        log::error!("Could not save the acknowledgement: {e}");
                    }
                    // original: acknowledgement changes refresh the notifier
                    if let Ok(config) = services.config.get_config() {
                        if let Ok(mut tray) = services.tray.lock() {
                            tray.update(&config);
                        }
                    }
                }
                "auth-token" => {
                    // original: AuthenticationTokenForm.ShowDialogOrFocus(project)
                    lock_state(&services.state).auth_token_target =
                        Some((server_url.clone(), project_url.clone()));
                    auth_token.set_token(
                        project.authentication_token.clone().unwrap_or_default().into(),
                    );
                    auth_token.set_cause(project.cause_text.clone().unwrap_or_default().into());
                    if let Err(e) = auth_token.show() {
                        log::error!("Could not show the authentication token dialog: {e}");
                    }
                }
                "claim-build" => {
                    // original: ClaimBuildForm.Initialize(project, lastFailedBuild)
                    let last_failed_number = project
                        .all_build_details
                        .as_ref()
                        .and_then(|details| details.last_failed_build.as_ref())
                        .map(|build| build.number);
                    let Some(last_failed_number) = last_failed_number else {
                        // original: return when LastFailedBuild == null
                        return;
                    };
                    lock_state(&services.state).claim_target =
                        Some((server_url.clone(), project_url.clone(), last_failed_number));
                    claim_build.set_claim_label(format!("Claim build: {last_failed_number}").into());
                    claim_build.set_reason("".into());
                    claim_build.set_sticky(false);
                    if let Err(e) = claim_build.show() {
                        log::error!("Could not show the claim build dialog: {e}");
                    }
                }
                "remove-project" => {
                    // original: ConfigurationService.RemoveProject(project)
                    let mut project = project.clone();
                    project.server = Some(server_url.clone());
                    if let Err(e) = services.config.remove_project(&project) {
                        log::error!("Could not remove the project: {e}");
                        return;
                    }
                    ui.set_selected_row(-1);
                    ui.set_selected_ack_project(false);
                    ui.set_selected_ack_status(false);
                    on_configuration_updated(&services, &ui);
                }
                other => log::warn!("Unknown menu action: {other}"),
            }
        }
    });

    // --- Settings window ---------------------------------------------------

    // "Add server" (original: ServerListControl.addServerButtonItem_ItemClick).
    {
        let services = services.clone();
        let edit_server = windows.edit_server.clone();
        windows.settings.on_add_server(move || {
            // original: new EditServerForm()
            lock_state(&services.state).edit_server = EditServerContext { index: None };
            edit_server.set_url("".into());
            edit_server.set_display_name("".into());
            edit_server.set_auth_enabled(false);
            edit_server.set_username("".into());
            edit_server.set_password("".into());
            edit_server.set_ignore_certificate(false);
            if let Err(e) = edit_server.show() {
                log::error!("Could not show the server dialog: {e}");
            }
        });
    }

    // "Edit server" (original: EditSelectedServer -> new EditServerForm(server)).
    {
        let services = services.clone();
        let edit_server = windows.edit_server.clone();
        windows.settings.on_edit_server(move |index| {
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(e) => {
                    log::error!("Could not read the configuration: {e}");
                    return;
                }
            };
            let Some(server) = config.servers.get(index as usize) else {
                return;
            };
            lock_state(&services.state).edit_server = EditServerContext {
                index: Some(index as usize),
            };
            edit_server.set_url(server.url.clone().into());
            edit_server.set_display_name(server.display_name.clone().into());
            edit_server.set_username(server.username.clone().unwrap_or_default().into());
            edit_server.set_password(server.password.clone().unwrap_or_default().into());
            edit_server.set_auth_enabled(server.username.is_some());
            edit_server.set_ignore_certificate(server.ignore_untrusted_certificate);
            if let Err(e) = edit_server.show() {
                log::error!("Could not show the server dialog: {e}");
            }
        });
    }

    // "Remove server" (original: RemoveSelectedServer).
    {
        let services = services.clone();
        let ui = ui.as_weak();
        let settings = windows.settings.as_weak();
        windows.settings.on_remove_server(move |index| {
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(e) => {
                    log::error!("Could not read the configuration: {e}");
                    return;
                }
            };
            let Some(server) = config.servers.get(index as usize) else {
                return;
            };
            if let Err(e) = services.config.remove_server(server) {
                log::error!("Could not remove the server: {e}");
                return;
            }
            lock_state(&services.state)
                .settings_projects
                .remove(&server.url);
            if let Some(settings) = settings.upgrade() {
                if let Err(e) = populate_settings(&services, &settings) {
                    log::error!("Could not reload the settings dialog: {e}");
                }
            }
            if let Some(ui) = ui.upgrade() {
                on_configuration_updated(&services, &ui);
            }
        });
    }

    // Selecting a server (original: serversGridView_FocusedRowChanged ->
    // Controller.UpdateProjectList -> JenkinsService.LoadProjects).
    {
        let services = services.clone();
        let settings = windows.settings.as_weak();
        windows.settings.on_server_selection_changed(move |index| {
            let settings = settings.unwrap();
            if let Err(e) = populate_project_list(&services.config, &settings, &services.state, index)
            {
                log::error!("Could not load the project list: {e}");
            }
            if index < 0 {
                return;
            }
            // live job list for the newly selected server
            if let Ok(config) = services.config.get_config() {
                if let Some(server) = config.servers.get(index as usize) {
                    spawn_project_load(&services, &settings, server);
                }
            }
        });
    }

    // Checking/unchecking a project
    // (original: ProjectListControl -> ConfigurationService.AddProject/RemoveProject).
    {
        let services = services.clone();
        let ui = ui.as_weak();
        let settings = windows.settings.as_weak();
        windows.settings.on_toggle_project(move |index, selected| {
            let settings = settings.unwrap();
            let server_index = settings.get_selected_server();
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(e) => {
                    log::error!("Could not read the configuration: {e}");
                    return;
                }
            };
            let Some(server) = config.servers.get(server_index as usize) else {
                return;
            };
            let project = {
                let state = lock_state(&services.state);
                state
                    .settings_projects
                    .get(&server.url)
                    .and_then(|projects| projects.get(index as usize))
                    .cloned()
            };
            let Some(mut project) = project else {
                return;
            };
            project.server = Some(server.url.clone());

            let result = if selected {
                services.config.add_project(&project)
            } else {
                services.config.remove_project(&project)
            };
            if let Err(e) = result {
                log::error!("Could not update the project list: {e}");
                return;
            }

            // original: projectsGridView.RefreshData()
            if let Err(e) = populate_project_list(&services.config, &settings, &services.state, server_index) {
                log::error!("Could not reload the project list: {e}");
            }
            if let Some(ui) = ui.upgrade() {
                on_configuration_updated(&services, &ui);
            }
        });
    }

    // "Select/unselect all projects" (original: SelectAllProjects/UnselectAllProjects).
    {
        let services = services.clone();
        let ui = ui.as_weak();
        let settings = windows.settings.as_weak();
        windows.settings.on_toggle_all_projects(move || {
            let settings = settings.unwrap();
            let server_index = settings.get_selected_server();
            let config = match services.config.get_config() {
                Ok(config) => config,
                Err(e) => {
                    log::error!("Could not read the configuration: {e}");
                    return;
                }
            };
            let Some(server) = config.servers.get(server_index as usize) else {
                return;
            };
            let snapshot = {
                let state = lock_state(&services.state);
                state
                    .settings_projects
                    .get(&server.url)
                    .cloned()
                    .unwrap_or_default()
            };
            if snapshot.is_empty() {
                return;
            }
            let mut projects = snapshot;
            for project in &mut projects {
                project.server = Some(server.url.clone());
            }

            // original: if (server.Projects.Count < dataSource.Count) SelectAll
            let all_selected = server.projects.len() >= projects.len();
            let result = if all_selected {
                services.config.remove_projects(&projects)
            } else {
                services.config.add_projects(&projects)
            };
            if let Err(e) = result {
                log::error!("Could not update the project list: {e}");
                return;
            }

            if let Err(e) = populate_project_list(&services.config, &settings, &services.state, server_index) {
                log::error!("Could not reload the project list: {e}");
            }
            if let Some(ui) = ui.upgrade() {
                on_configuration_updated(&services, &ui);
            }
        });
    }

    // Sound "Test" buttons (original: SettingsForm test buttons).
    {
        let services = services.clone();
        windows.settings.on_test_sound(move |key| {
            let key = key.to_string();
            let path = services
                .config
                .get_notification_settings()
                .ok()
                .and_then(|settings| {
                    settings
                        .sound_paths
                        .and_then(|paths| paths.get(&key).cloned())
                });
            match path {
                Some(path) if !path.is_empty() => {
                    if let Err(e) = services.notifications.play_sound(&path) {
                        log::error!("Could not play the sound: {e}");
                    }
                }
                _ => log::warn!("No sound file configured for '{key}'"),
            }
        });
    }

    // "Notifications" tab changes (original: SettingsForm NotificationSettings).
    {
        let services = services.clone();
        let ui = ui.as_weak();
        let settings = windows.settings.as_weak();
        windows.settings.on_save_notifications(move || {
            let settings = settings.unwrap();
            let mut notification_settings = match services.config.get_notification_settings() {
                Ok(settings) => settings,
                Err(e) => {
                    log::error!("Could not read the notification settings: {e}");
                    return;
                }
            };
            notification_settings.sound_notifications = settings.get_enable_sound();
            notification_settings.treat_unstable_as_failed = settings.get_treat_unstable_as_failed();
            notification_settings.balloon_notifications = settings.get_enable_balloon();

            let mut paths = notification_settings.sound_paths.take().unwrap_or_default();
            set_sound_path(&mut paths, "failed", settings.get_sound_failed().to_string());
            set_sound_path(&mut paths, "fixed", settings.get_sound_fixed().to_string());
            set_sound_path(
                &mut paths,
                "still_failing",
                settings.get_sound_still_failing().to_string(),
            );
            set_sound_path(
                &mut paths,
                "succeeded",
                settings.get_sound_succeeded().to_string(),
            );
            notification_settings.sound_paths = if paths.is_empty() {
                None
            } else {
                Some(paths)
            };

            if let Err(e) = services.config.set_notification_settings(notification_settings) {
                log::error!("Could not save the notification settings: {e}");
                return;
            }
            // original: ConfigurationUpdated -> NotificationService.Execute +
            // TrayNotifier.UpdateNotifier (sound/balloon settings changed).
            if let Some(ui) = ui.upgrade() {
                on_configuration_updated(&services, &ui);
            }
        });
    }

    // "General" tab changes (original: SettingsForm GeneralSettings).
    {
        let services = services.clone();
        let ui = ui.as_weak();
        let settings = windows.settings.as_weak();
        windows.settings.on_save_general(move || {
            let settings = settings.unwrap();
            let mut general = match services.config.get_general_settings() {
                Ok(general) => general,
                Err(e) => {
                    log::error!("Could not read the general settings: {e}");
                    return;
                }
            };
            general.refresh_interval_in_seconds = settings.get_refresh_interval().max(1) as u64;
            general.update_main_window_icon = settings.get_update_main_window_icon();
            general.integrate_with_claim_plugin = settings.get_integrate_with_claim_plugin();
            general.show_project_display_name = settings.get_show_project_display_name();
            general.check_for_updates = settings.get_check_for_updates();
            let check_for_updates = general.check_for_updates;

            if let Err(e) = services.config.set_general_settings(general) {
                log::error!("Could not save the general settings: {e}");
                return;
            }
            // original: SettingsForm OK -> ApplicationUpdateService.
            // EnableTimer(checked)
            services.app_update.clone().enable_timer(check_for_updates);
            // original: ConfigurationUpdated -> MainForm refreshes the grid
            // (display names), the claim plugin columns, the notification
            // service and the tray notifier.
            if let Some(ui) = ui.upgrade() {
                on_configuration_updated(&services, &ui);
            }
        });
    }

    // --- Edit Server dialog ------------------------------------------------

    {
        let services = services.clone();
        let ui = ui.as_weak();
        let settings = windows.settings.as_weak();
        let edit_server = windows.edit_server.as_weak();
        windows.edit_server.on_accepted(move || {
            let edit_server = edit_server.unwrap();
            // original: ValidateForm / urlTextBox_Leave
            let raw_url = edit_server.get_url().to_string();
            if !is_valid_server_url(&raw_url) {
                log::warn!("The server URL is invalid");
                return; // original: the Validate button stays disabled
            }
            let url = normalize_server_url(&raw_url);
            let display_name = edit_server.get_display_name().to_string();
            let (username, password) = if edit_server.get_auth_enabled() {
                (
                    Some(edit_server.get_username().to_string()),
                    Some(edit_server.get_password().to_string()),
                )
            } else {
                (None, None)
            };
            let ignore_certificate = edit_server.get_ignore_certificate();

            let context = lock_state(&services.state).edit_server.clone();
            let result = match context.index {
                // original: ConfigurationService.AddServer
                None => services.config.add_server(
                    &url,
                    &display_name,
                    username.as_deref(),
                    password.as_deref(),
                    ignore_certificate,
                )
                .map(|_| ()),
                // original: ConfigurationService.UpdateServer
                Some(index) => {
                    let config = match services.config.get_config() {
                        Ok(config) => config,
                        Err(e) => {
                            log::error!("Could not read the configuration: {e}");
                            return;
                        }
                    };
                    let Some(server) = config.servers.get(index) else {
                        return;
                    };
                    let old_url = server.url.clone();
                    let result = services.config.update_server(
                        &old_url,
                        &url,
                        &display_name,
                        username.as_deref(),
                        password.as_deref(),
                        ignore_certificate,
                    );
                    if result.is_ok() && old_url != url {
                        // re-key the settings snapshot after a URL change
                        let mut state = lock_state(&services.state);
                        if let Some(projects) = state.settings_projects.remove(&old_url) {
                            state.settings_projects.insert(url.clone(), projects);
                        }
                    }
                    result
                }
            };
            if let Err(e) = result {
                log::error!("Could not save the server: {e}");
                return;
            }

            if let Err(e) = edit_server.hide() {
                log::error!("Could not close the server dialog: {e}");
            }
            // original: serversGridView.RefreshData() + UpdateProjectList(server)
            if let Some(settings) = settings.upgrade() {
                if let Err(e) = populate_settings(&services, &settings) {
                    log::error!("Could not reload the settings dialog: {e}");
                }
            }
            if let Some(ui) = ui.upgrade() {
                on_configuration_updated(&services, &ui);
            }
        });
    }

    {
        let edit_server = windows.edit_server.as_weak();
        windows.edit_server.on_canceled(move || {
            if let Err(e) = edit_server.unwrap().hide() {
                log::error!("Could not close the server dialog: {e}");
            }
        });
    }

    // --- Authentication token dialog --------------------------------------

    {
        let services = services.clone();
        let auth_token = windows.auth_token.as_weak();
        windows.auth_token.on_accepted(move || {
            let auth_token = auth_token.unwrap();
            // original: AuthenticationTokenForm "Validate" button
            let token = auth_token.get_token().to_string();
            let cause = auth_token.get_cause().to_string();
            let token = token.trim();
            let cause = cause.trim();
            let target = lock_state(&services.state).auth_token_target.clone();
            if let Some((server_url, project_url)) = target {
                let result = services.config.set_project_authentication(
                    &server_url,
                    &project_url,
                    if token.is_empty() { None } else { Some(token) },
                    if cause.is_empty() { None } else { Some(cause) },
                );
                if let Err(e) = result {
                    log::error!("Could not save the authentication token: {e}");
                }
            }
            if let Err(e) = auth_token.hide() {
                log::error!("Could not close the authentication token dialog: {e}");
            }
        });
    }

    {
        let auth_token = windows.auth_token.as_weak();
        windows.auth_token.on_canceled(move || {
            if let Err(e) = auth_token.unwrap().hide() {
                log::error!("Could not close the authentication token dialog: {e}");
            }
        });
    }

    // --- Claim build dialog ------------------------------------------------

    {
        let services = services.clone();
        let claim_build = windows.claim_build.as_weak();
        let message_box = windows.message_box.as_weak();
        windows.claim_build.on_accepted(move || {
            let claim_build = claim_build.unwrap();
            let reason = claim_build.get_reason().to_string();
            let sticky = claim_build.get_sticky();
            let target = lock_state(&services.state).claim_target.clone();
            if let Some((server_url, project_url, build_number)) = target {
                // original: ClaimService.ClaimBuild(project, lastFailedBuild,
                // reason, sticky)
                match services.config.get_config() {
                    Err(e) => log::error!("Could not read the configuration: {e}"),
                    Ok(config) => {
                        if let Some((server, project)) =
                            project_in(&config, &server_url, &project_url)
                        {
                            let server = server.clone();
                            let project = project.clone();
                            let cache = services.updater.borrow().cache();
                            let message_box = message_box.clone();
                            services.runtime.spawn(async move {
                                let name = project.name.clone();
                                if let Err(e) = jenkins_client::claim_build(
                                    &server,
                                    &project,
                                    build_number,
                                    &reason,
                                    sticky,
                                    &cache,
                                )
                                .await
                                {
                                    // original: MessageBox.Show(webEx.Message,
                                    // "Claim failed", OK, Error) in
                                    // ClaimService.ClaimBuild (WebException).
                                    let cause = errors::root_cause(&e);
                                    log::error!("Claim failed for {name}: {cause}");
                                    show_message_box(&message_box, "Claim failed", &cause);
                                } else {
                                    log::info!("Claimed build {build_number} of {name}");
                                }
                            });
                        }
                    }
                }
            }
            if let Err(e) = claim_build.hide() {
                log::error!("Could not close the claim build dialog: {e}");
            }
        });
    }

    {
        let claim_build = windows.claim_build.as_weak();
        windows.claim_build.on_canceled(move || {
            if let Err(e) = claim_build.unwrap().hide() {
                log::error!("Could not close the claim build dialog: {e}");
            }
        });
    }

    // --- Build with parameters dialog (new feature) ------------------------
    {
        // Every edit updates the authoritative name/value list; the dialog
        // itself only renders the initial defaults.
        let state = services.state.clone();
        windows.build_params.on_param_edited(move |index, value| {
            let mut state = lock_state(&state);
            if let Some(entry) = state.build_params.get_mut(index as usize) {
                entry.1 = value.to_string();
            }
        });
    }

    {
        let services = services.clone();
        let build_params = windows.build_params.as_weak();
        let message_box = windows.message_box.as_weak();
        windows.build_params.on_accepted(move || {
            let dialog = build_params.unwrap();
            let (target, values) = {
                let mut state = lock_state(&services.state);
                (
                    state.build_params_target.take(),
                    std::mem::take(&mut state.build_params),
                )
            };
            if let Some((server_url, project_url)) = target {
                // New feature: there is no counterpart in the original; the
                // plain counterpart is the "run-build" menu action above
                // (`JenkinsService.SafeRunBuild`).
                if let Ok(config) = services.config.get_config() {
                    if let Some((server, project)) =
                        project_in(&config, &server_url, &project_url)
                    {
                        let client = services.clients.for_server(server).clone();
                        let server = server.clone();
                        let project = project.clone();
                        let name = project.name.clone();
                        let message_box = message_box.clone();
                        services.runtime.spawn(async move {
                            if let Err(e) = jenkins_client::trigger_build_parameters(
                                &client, &server, &project, &values,
                            )
                            .await
                            {
                                // New feature: no original counterpart; the
                                // error box matches the plain RunBuild's
                                // (`RunBuildFailed_Text`).
                                let cause = errors::root_cause(&e);
                                log::error!(
                                    "Failed to trigger a parameterized build of {name}: {cause}"
                                );
                                show_message_box(
                                    &message_box,
                                    "Error",
                                    &format!("Failed running build: {cause}."),
                                );
                            } else {
                                log::info!("Build triggered for {name}");
                            }
                        });
                    }
                }
            }
            if let Err(e) = dialog.hide() {
                log::error!("Could not close the build parameters dialog: {e}");
            }
        });
    }

    {
        let build_params = windows.build_params.as_weak();
        let state = services.state.clone();
        windows.build_params.on_canceled(move || {
            {
                let mut state = lock_state(&state);
                state.build_params_target = None;
                state.build_params.clear();
            }
            if let Err(e) = build_params.unwrap().hide() {
                log::error!("Could not close the build parameters dialog: {e}");
            }
        });
    }

    // --- Message box (original: XtraMessageBox OK / Yes / No buttons) ------

    {
        let message_box = windows.message_box.as_weak();
        windows.message_box.on_accepted(move || {
            // A registered action means this is the update prompt's "Yes"
            // (original: ApplicationUpdateHandler accepts the new version).
            if let Some(action) = take_message_box_yes_action() {
                action();
            }
            if let Err(e) = message_box.unwrap().hide() {
                log::error!("Could not close the message box: {e}");
            }
        });
    }

    {
        let message_box = windows.message_box.as_weak();
        windows.message_box.on_declined(move || {
            if take_message_box_yes_action().is_some() {
                log::info!("Update refused by user");
            }
            if let Err(e) = message_box.unwrap().hide() {
                log::error!("Could not close the message box: {e}");
            }
        });
    }

    // --- About dialog ------------------------------------------------------

    {
        let about = windows.about.clone();
        about.on_open_link(move |url| open_url(&url));
    }
}

// ---------------------------------------------------------------------------
// Tray events
// ---------------------------------------------------------------------------

/// Shows or restores the main window (original: `MainForm.ShowOrFocus`).
///
/// Documented deviation: Slint offers no API to raise or focus an already
/// visible window, so this un-minimizes and (re-)shows the window instead.
fn show_or_focus(ui: &AppWindow) {
    let window = ui.window();
    window.set_minimized(false);
    if let Err(e) = window.show() {
        log::error!("Could not show the main window: {e}");
    }
}

/// Handles a tray click/menu selection on the UI thread (original:
/// `TrayNotifier`: `notifyIcon_MouseDoubleClick`, the Open / Settings /
/// Refresh / About / Exit menu `Click` handlers).
fn handle_tray_event(
    event: tray::TrayEvent,
    ui: &AppWindow,
    services: &Services,
    windows: &Windows,
) {
    match event {
        // original: notifyIcon_MouseDoubleClick -> ShowOrFocus (single click
        // is deactivated in TrayNotifier; Linux SNI has no double click).
        tray::TrayEvent::OpenRequested | tray::TrayEvent::Menu(1) => show_or_focus(ui),
        // original: settingsMenuItem_Click -> SettingsForm.ShowDialog
        tray::TrayEvent::Menu(2) => {
            if let Err(e) = populate_settings(services, &windows.settings) {
                log::error!("Could not load the settings dialog: {e}");
            }
            if let Err(e) = windows.settings.show() {
                log::error!("Could not show the settings dialog: {e}");
            }
        }
        // original: refreshMenuItem_Click -> ProjectsUpdateService.UpdateProjects
        tray::TrayEvent::Menu(3) => {
            services.updater.borrow().refresh_all();
            ui.set_status_text("".into());
            if let Err(e) = refresh_rows(ui, &services.config, &services.state) {
                log::error!("Could not refresh the projects grid: {e}");
            }
        }
        // original: aboutMenuItem_Click -> AboutForm.ShowDialog
        tray::TrayEvent::Menu(4) => {
            if let Err(e) = windows.about.show() {
                log::error!("Could not show the About dialog: {e}");
            }
        }
        // original: exitToolStripMenuItem_Click -> Application.Exit
        tray::TrayEvent::Menu(5) => {
            if let Err(e) = slint::quit_event_loop() {
                log::error!("Could not quit the event loop: {e}");
            }
        }
        other => log::warn!("Unknown tray event: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    env_logger::init();
    log::info!("Jenkins Tray starting");

    // Tokio drives the async services (original: BackgroundProcessExecutor).
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    // --- Services (original: Spring context object graph) ---

    let config_service = Arc::new(ConfigurationService::new()?);

    // Tray (original: NotifyIcon + context menu + UpdateNotifierStartup).
    let tray_hub = Arc::new(Mutex::new(tray::TrayHub::new()?));

    let mut notification_service = NotificationService::new();
    notification_service.set_tray_notifier(Box::new(tray::StubTrayBackend));
    let notification_service = Arc::new(notification_service);

    let clients = jenkins_client::Clients::new()?;

    let updater = ProjectsUpdateService::new(
        UpdaterConfig::default(),
        clients.clone(),
        config_service.clone(),
    );
    let updater = Rc::new(RefCell::new(updater));

    // --- Windows (created once and reused, like the original forms) ---

    let ui = AppWindow::new()?;
    let windows = Windows {
        settings: Rc::new(SettingsWindow::new()?),
        edit_server: Rc::new(EditServerWindow::new()?),
        about: Rc::new(AboutWindow::new()?),
        auth_token: Rc::new(AuthTokenWindow::new()?),
        claim_build: Rc::new(ClaimBuildWindow::new()?),
        build_params: Rc::new(BuildParamsWindow::new()?),
        message_box: Rc::new(MessageBoxWindow::new()?),
    };
    windows
        .about
        .set_version(format!("Version {}", env!("CARGO_PKG_VERSION")).into());

    // Application update service (original: the Spring-managed
    // ApplicationUpdateService + ApplicationUpdateHandler singletons).
    let app_update = ApplicationUpdateService::new(runtime.handle().clone(), config_service.clone());
    app_update.on_new_version(Arc::new({
        let message_box = windows.message_box.as_weak();
        move |version: &str, installer_url: &str| {
            // original: ApplicationUpdateHandler.applicationUpdateService_
            // NewVersionAvailable -> Yes/No box; Yes opens the installer page
            // and exits the application.
            let message = format!(
                "A new version is available: v{version}.\n\n\
                 Do you want to download the update now (the application will close) ?"
            );
            let installer_url = installer_url.to_string();
            show_yes_no_message_box(
                &message_box,
                "Jenkins Tray updates",
                &message,
                Box::new(move || {
                    log::info!("Update accepted by user");
                    open_url(&installer_url);
                    // original: Application.Exit()
                    if let Err(e) = slint::quit_event_loop() {
                        log::error!("Could not quit the event loop: {e}");
                    }
                }),
            );
        }
    }));

    let services = Services {
        config: config_service,
        notifications: notification_service,
        clients,
        updater,
        state: Arc::new(Mutex::new(AppState::default())),
        tray: tray_hub,
        app_update,
        runtime: runtime.handle().clone(),
    };

    // original: ApplicationUpdateService.Initialize -> EnableTimer(config) -
    // the first check runs 5 seconds after start-up, then hourly.
    services.app_update.clone().initialize();

    // Initial grid fill (original: MainForm constructor -> LoadProjects).
    refresh_rows(&ui, &services.config, &services.state)?;
    wire_callbacks(&ui, &services, &windows);

    // Close to the tray (original: JenkinsTrayForm_FormClosing -> e.Cancel =
    // true; Hide()). Windows always shows the NotifyIcon; without a tray host
    // (e.g. under WSLg) hiding would strand the app, so closing quits instead.
    ui.window().on_close_requested({
        let tray = services.tray.clone();
        move || {
            let has_tray = tray
                .lock()
                .map(|tray| tray.can_show_icon())
                .unwrap_or(false);
            log::info!("Window close requested (tray icon visible: {has_tray})");
            if has_tray {
                return CloseRequestResponse::HideWindow;
            }
            log::info!(
                "No tray icon available: closing the window exits \
                 (the original hides to the tray)"
            );
            if let Err(e) = slint::quit_event_loop() {
                log::error!("Could not quit the event loop: {e}");
            }
            CloseRequestResponse::HideWindow
        }
    });

    // Tray clicks/menus (original: the TrayNotifier event handlers) arrive on
    // a channel and are drained on the UI thread by a Slint timer.
    let (tray_events_tx, tray_events_rx) = std::sync::mpsc::channel::<tray::TrayEvent>();
    if let Ok(mut tray) = services.tray.lock() {
        tray.set_event_sender(tray_events_tx);
    }
    let _tray_events_timer = slint::Timer::default();
    _tray_events_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(100),
        {
            let ui = ui.as_weak();
            let services = services.clone();
            let windows = windows.clone();
            move || {
                let Some(ui) = ui.upgrade() else {
                    return;
                };
                while let Ok(event) = tray_events_rx.try_recv() {
                    handle_tray_event(event, &ui, &services, &windows);
                }
            }
        },
    );


    // ProjectsUpdated event (original: MainForm.OnProjectsUpdated,
    // TrayNotifier.updateService_ProjectsUpdated and
    // NotificationService.Execute all subscribe to ProjectsUpdated).
    services.updater.borrow_mut().set_on_updated({
        let ui = ui.as_weak();
        let config_service = services.config.clone();
        let notifications = services.notifications.clone();
        let tray = services.tray.clone();
        let state = services.state.clone();
        Arc::new(move || {
            let ui = ui.clone();
            let config_service = config_service.clone();
            let notifications = notifications.clone();
            let tray = tray.clone();
            let state = state.clone();
            if let Err(e) = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui.upgrade() else {
                    return;
                };
                // original: UpdateProjects -> RefreshData + UpdateStatusBar
                if let Err(e) = refresh_rows(&ui, &config_service, &state) {
                    log::error!("Could not refresh the projects grid: {e}");
                }
                if let Ok(config) = config_service.get_config() {
                    // original: NotificationService.Execute
                    if let Err(e) = notifications.execute(&config) {
                        log::error!("Could not run the notification service: {e}");
                    }
                    // original: TrayNotifier.UpdateNotifier
                    if let Ok(mut tray) = tray.lock() {
                        tray.update(&config);
                    }
                }
            }) {
                log::error!("Could not dispatch the projects-updated event: {e}");
            }
        })
    });
    services.updater.borrow_mut().start(runtime.handle());

    let run_result = ui.run();

    // --- Cleanup (original: application exit) ---
    runtime.block_on(services.updater.borrow_mut().stop());
    // original: Program.Application_Exit -> ConfigurationService.SaveConfiguration
    let _ = services.config.save();
    if let Ok(tray) = services.tray.lock() {
        let _ = tray.backend.exit();
    }
    run_result?;
    log::info!("Jenkins Tray stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_header_clicks_cycle_ascending_descending_unsorted() {
        // first click: ascending (matches the initial SortInfo direction)
        let asc = next_sort(None, SortKey::LastFailure);
        assert_eq!(asc, Some(GridSort { key: SortKey::LastFailure, ascending: true }));
        // second click: descending
        let desc = next_sort(asc, SortKey::LastFailure);
        assert_eq!(desc, Some(GridSort { key: SortKey::LastFailure, ascending: false }));
        // third click: no sort at all
        assert_eq!(next_sort(desc, SortKey::LastFailure), None);
    }

    #[test]
    fn clicking_another_column_sorts_it_ascending() {
        let other = next_sort(
            Some(GridSort { key: SortKey::Name, ascending: false }),
            SortKey::Details,
        );
        assert_eq!(other, Some(GridSort { key: SortKey::Details, ascending: true }));
    }
}
