//! Tray backend abstraction and tray notifier state.
//!
//! Port of C# `UI/TrayNotifier.cs`: the worst-status computation, the tooltip
//! text, the error/regression balloons and the acknowledged-status store live
//! in [`TrayNotifierState`]; the platform rendering goes through
//! [`TrayBackend`] - a StatusNotifierItem ([`ksni`]) on Linux, a
//! `Shell_NotifyIconW` backend on Windows ([`win32_backend`]) and
//! [`StubTrayBackend`] elsewhere, where every call degrades to logging.
//!
//! The Linux tray is opportunistic only (Windows is the primary target per
//! the original app): without a StatusNotifierWatcher - e.g. under WSLg -
//! no icon is shown, so reopening the window falls back to relaunching the
//! application, which asks the running instance to show it (see
//! [`crate::single_instance`]).

use std::collections::HashMap;

use anyhow::Result;

use crate::model::{AllBuildDetails, BuildStatus, BuildStatusEnum, Configuration, Project};
use crate::notifications::TrayNotifier;

#[cfg(all(unix, not(target_os = "macos")))]
mod ksni_backend;
#[cfg(windows)]
mod win32_backend;

/// Maximum tooltip length (original: TrayNotifier.MAX_TOOLTIP_LENGTH).
pub const MAX_TOOLTIP_LENGTH: usize = 127;

/// Icon severity of a balloon notification (original: ToolTipIcon). The
/// original shows balloons with a 3000 ms timeout (error/regression) and a
/// 10000 ms timeout (activity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// Used by the (not yet ported) activity balloons.
    #[allow(dead_code)]
    Info,
    Warning,
    Error,
}

/// Something the user did on the tray icon (original: the
/// `notifyIcon_MouseDoubleClick` / context-menu `Click` handlers in
/// `TrayNotifier`). Backends push these into a channel that main.rs drains on
/// the UI thread, so no backend ever touches Slint from a foreign thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// A tray context menu entry was clicked (see [`TRAY_MENU_ITEMS`] ids).
    Menu(u32),
    /// The tray icon was clicked (original: `notifyIcon_MouseDoubleClick` ->
    /// `ShowOrFocus`; the original's single click is deactivated).
    OpenRequested,
}

/// Channel sending side for [`TrayEvent`]s.
pub type TrayEventSender = std::sync::mpsc::Sender<TrayEvent>;

/// Trait for platform-specific tray implementations.
pub trait TrayBackend: Send + Sync {
    fn initialize(&mut self) -> Result<()>;
    /// True when the tray icon is actually visible (a host is showing it).
    /// Used by the close handler: hiding to an invisible icon would strand
    /// the user with no way back except relaunching.
    fn can_show_icon(&self) -> bool;
    /// Replace the tray icon (original: notifyIcon.Icon = iconsByKey[key]).
    fn update_icon(&self, status: &BuildStatus) -> Result<()>;
    /// Replace the tray tooltip (original: SetNotifyIconText).
    fn set_tooltip(&self, text: &str) -> Result<()>;
    /// Show a balloon notification (original: notifyIcon.ShowBalloonTip).
    fn show_notification(
        &self,
        title: &str,
        message: &str,
        kind: NotificationKind,
    ) -> Result<()>;
    fn set_context_menu(&self, items: Vec<MenuItem>) -> Result<()>;
    /// Delivers tray clicks/menu selections to the UI thread.
    fn set_event_sender(&mut self, sender: TrayEventSender);
    fn exit(&self) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    pub enabled: bool,
    /// Only rendered by the Windows backend - the original tray menu has no
    /// checkmarks and SNI `StandardItem`s cannot show one.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub checked: bool,
}

/// Tray context menu entries in the original order
/// (original: NotifyIcon context menu Open / Settings / Refresh / About / Exit).
pub const TRAY_MENU_ITEMS: &[(&str, u32)] = &[
    ("Open", 1),
    ("Settings", 2),
    ("Refresh", 3),
    ("About", 4),
    ("Exit", 5),
];

pub struct StubTrayBackend;

impl TrayBackend for StubTrayBackend {
    fn initialize(&mut self) -> Result<()> {
        Ok(())
    }
    fn can_show_icon(&self) -> bool {
        false
    }
    fn update_icon(&self, status: &BuildStatus) -> Result<()> {
        log::info!("Tray icon: {}", status.key());
        Ok(())
    }
    fn set_tooltip(&self, text: &str) -> Result<()> {
        log::debug!("Tray tooltip: {text}");
        Ok(())
    }
    fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()> {
        log::info!("Balloon ({kind:?}): {title} - {}", message.replace('\n', " | "));
        Ok(())
    }
    fn set_context_menu(&self, _items: Vec<MenuItem>) -> Result<()> {
        Ok(())
    }
    fn set_event_sender(&mut self, sender: TrayEventSender) {
        // No tray to click: drop the sender (original: no NotifyIcon either
        // when the backend is a stub).
        drop(sender);
    }
    fn exit(&self) -> Result<()> {
        Ok(())
    }
}

impl TrayNotifier for StubTrayBackend {
    fn play_sound(&self, path: &str) -> Result<()> {
        log::info!("Playing sound: {path}");
        Ok(())
    }
}

/// The platform tray backend (original: `NotifyIcon` in `TrayNotifier`).
///
/// Linux gets a StatusNotifierItem ([`ksni_backend`]), Windows a
/// `Shell_NotifyIconW` implementation ([`win32_backend`]); everything else
/// falls back to the logging stub.
pub fn create_tray_backend() -> Box<dyn TrayBackend> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Box::new(ksni_backend::KsniBackend::new())
    }
    #[cfg(windows)]
    {
        Box::new(win32_backend::Win32Backend::new())
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        Box::new(StubTrayBackend)
    }
}

// ---------------------------------------------------------------------------
// Tray icon rendering
// ---------------------------------------------------------------------------

/// Draws the status ball as straight-alpha RGBA pixels - the vector
/// replacement for the original `StatusIcons/*.gif` tray icons, using the
/// same palette as the main grid's `StatusBall`.
pub fn status_icon_rgba(status: &BuildStatus, size: u32) -> Vec<u8> {
    let (r, g, b) = match status.value {
        BuildStatusEnum::Failed => (0xe5u8, 0x39, 0x35),
        BuildStatusEnum::Successful => (0x43, 0x9f, 0xe0),
        BuildStatusEnum::Unstable => (0xfb, 0x8c, 0x00),
        BuildStatusEnum::Aborted => (0x9e, 0x9e, 0x9e),
        BuildStatusEnum::Disabled => (0x61, 0x61, 0x61),
        BuildStatusEnum::Indeterminate => (0xbd, 0xbd, 0xbd),
        BuildStatusEnum::Unknown => (0x8e, 0x24, 0xaa),
    };
    let s = size as f32;
    let center = s / 2.0;
    let radius = center - 0.5;
    // like the grid: 1px border on a 16px ball, scaled up
    let border = (s / 16.0).max(1.0);
    // #000000 when stuck, #ffffffcc otherwise (grid StatusBall)
    let (border_col, border_alpha): ([f32; 3], f32) = if status.is_stuck {
        ([0.0, 0.0, 0.0], 1.0)
    } else {
        ([1.0, 1.0, 1.0], 0.8)
    };

    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            // anti-aliased edge of the disc
            let cover = (radius + 0.5 - dist).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }

            let (mut col, mut alpha) = if dist > radius - border {
                (border_col, border_alpha)
            } else {
                (
                    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
                    1.0,
                )
            };
            // Static white ring while a build is in progress (the grid pulses,
            // the original tray GIFs animate - a single frame here).
            if status.is_in_progress {
                let ring_mid = radius - border - radius * 0.3;
                let ring_width = (s * 0.09).max(1.0);
                if (dist - ring_mid).abs() < ring_width {
                    col = [1.0, 1.0, 1.0];
                    alpha = 0.75;
                }
            }

            let a = cover * alpha;
            let i = ((y * size + x) * 4) as usize;
            out[i] = (col[0] * 255.0 * a).round() as u8;
            out[i + 1] = (col[1] * 255.0 * a).round() as u8;
            out[i + 2] = (col[2] * 255.0 * a).round() as u8;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tray notifier state (original: TrayNotifier)
// ---------------------------------------------------------------------------

/// The in-memory state the original keeps in `TrayNotifier`.
#[derive(Default)]
pub struct TrayNotifierState {
    /// Acknowledged status per project URL
    /// (original: acknowledgedStatusByProject).
    acknowledged_status_by_project: HashMap<String, BuildStatus>,
    /// Overall status of the previous update (original: lastBuildStatus).
    last_build_status: Option<BuildStatus>,
    /// Build details of the previous update per project URL (original:
    /// lastProjectsBuildDetails; the original stores `null` for projects
    /// without details).
    last_projects_build_details: HashMap<String, Option<AllBuildDetails>>,
}

impl TrayNotifierState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initial icon/tooltip (original: TrayNotifier.UpdateNotifierStartup).
    pub fn startup(&self, backend: &dyn TrayBackend) -> Result<()> {
        backend.update_icon(&BuildStatus::unknown())?;
        backend.set_tooltip("JenkinsTray")?;
        Ok(())
    }

    /// Recompute icon, tooltip and balloons
    /// (original: TrayNotifier.UpdateNotifier).
    pub fn update(&mut self, config: &Configuration, backend: &dyn TrayBackend) {
        if let Err(e) = self.do_update(config, backend) {
            log::error!("Could not update the tray notifier: {e}");
            let _ = backend.update_icon(&BuildStatus::unknown());
        }
    }

    fn do_update(&mut self, config: &Configuration, backend: &dyn TrayBackend) -> Result<()> {
        let mut worst_build_status: Option<BuildStatusEnum> = None;
        let mut build_in_progress = false;
        let mut build_is_stuck = false;
        let mut error_projects: Vec<&Project> = Vec::new();
        let mut regressing_projects: Vec<&Project> = Vec::new();
        let mut progressing_and_error_projects: Vec<&Project> = Vec::new();
        let mut total_project_count = 0usize;

        for server in &config.servers {
            for project in &server.projects {
                total_project_count += 1;

                let status = self.get_project_status(project);
                if worst_build_status.is_none_or(|worst| status.value > worst) {
                    worst_build_status = Some(status.value);
                }
                if status.value >= BuildStatusEnum::Failed {
                    error_projects.push(project);
                }
                if status.value > BuildStatusEnum::Successful {
                    progressing_and_error_projects.push(project);
                }
                if status.is_in_progress {
                    build_in_progress = true;
                    progressing_and_error_projects.push(project);
                }
                if status.is_stuck {
                    build_is_stuck = true;
                }
                // original: IsRegressing reads the snapshot of the previous
                // update, which is written afterwards.
                if self.is_regressing(project) {
                    regressing_projects.push(project);
                }
                self.last_projects_build_details
                    .insert(project.url.clone(), project.all_build_details.clone());
            }
        }

        let overall = BuildStatus {
            value: worst_build_status.unwrap_or(BuildStatusEnum::Unknown),
            is_in_progress: build_in_progress,
            is_stuck: build_is_stuck,
        };

        // original: UpdateIcon - the main-window icon update
        // (GeneralSettings.UpdateMainWindowIcon) has no Slint equivalent yet.
        backend.update_icon(&overall)?;

        self.update_tray_tooltip(&progressing_and_error_projects, total_project_count, backend)?;

        if config.notification_settings.balloon_notifications {
            self.update_balloon_tip(&error_projects, &regressing_projects, backend)?;
            // original: ShowBallowTip(interestingProjects) - driven by
            // ProjectActivity, which is not ported yet.
            log::debug!("Activity balloons are not available yet (ProjectActivity not ported)");
        }

        self.last_build_status = Some(overall);
        Ok(())
    }

    /// Status of a project, honouring the acknowledgement
    /// (original: TrayNotifier.GetProjectStatus).
    fn get_project_status(&mut self, project: &Project) -> BuildStatus {
        let status = project.status();
        let acknowledged = self
            .acknowledged_status_by_project
            .get(&project.url)
            .cloned();

        if project.is_acknowledged || acknowledged.is_some() {
            if project.is_acknowledged
                || acknowledged
                    .as_ref()
                    .is_some_and(|acknowledged| acknowledged.value == status.value)
            {
                return BuildStatus {
                    value: BuildStatusEnum::Successful,
                    is_in_progress: false,
                    is_stuck: false,
                };
            }
            if status.value != BuildStatusEnum::Unknown
                && acknowledged.as_ref().is_some_and(|a| a.is_worse(&status))
            {
                // the status got better than the acknowledged one: stop
                // hiding it (original: ClearAcknowledgedStatus)
                self.acknowledged_status_by_project.remove(&project.url);
            }
        }
        status
    }

    /// Whether the project's status got worse since the previous update
    /// (original: TrayNotifier.IsRegressing).
    fn is_regressing(&self, project: &Project) -> bool {
        let Some(previous) = self.last_projects_build_details.get(&project.url) else {
            return false;
        };
        let Some(previous) = previous else {
            return false;
        };
        let Some(new_details) = project.all_build_details.as_ref() else {
            return false;
        };
        // moving from unknown/aborted to successful is not a regression
        if new_details.status.value <= BuildStatusEnum::Successful {
            return false;
        }
        new_details.status.is_worse(&previous.status)
    }

    /// Builds the tray tooltip (original: TrayNotifier.UpdateTrayTooltip).
    fn update_tray_tooltip(
        &mut self,
        progressing_and_error_projects: &[&Project],
        total_project_count: usize,
        backend: &dyn TrayBackend,
    ) -> Result<()> {
        let mut tooltip = String::new();

        if total_project_count == 0 {
            tooltip.push_str("No projects monitored.");
        } else if !progressing_and_error_projects.is_empty() {
            for project in progressing_and_error_projects {
                // original: acknowledged projects are skipped
                if project.is_acknowledged
                    || self.acknowledged_status_by_project.contains_key(&project.url)
                {
                    continue;
                }
                if !tooltip.is_empty() {
                    tooltip.push('\n');
                }
                let status = self.get_project_status(project);
                if status.is_in_progress && status.value == BuildStatusEnum::Failed {
                    tooltip.push_str(&format!("{}: Failed and in progress.", project.name));
                } else if status.is_in_progress {
                    tooltip.push_str(&format!("{}: In progress.", project.name));
                } else {
                    tooltip.push_str(&format!("{}: {:?}.", project.name, status.value));
                }
                if tooltip.chars().count() > MAX_TOOLTIP_LENGTH {
                    break;
                }
            }
        } else {
            tooltip.push_str("All builds are good.");
        }

        backend.set_tooltip(&truncate_tooltip(&tooltip))?;
        Ok(())
    }

    /// Error / regression balloons (original: TrayNotifier.UpdateBalloonTip).
    fn update_balloon_tip(
        &mut self,
        error_projects: &[&Project],
        regressing_projects: &[&Project],
        backend: &dyn TrayBackend,
    ) -> Result<()> {
        if self.last_build_status.as_ref().is_some_and(|last| last.value < BuildStatusEnum::Failed)
            && !error_projects.is_empty()
        {
            let mut text = String::new();
            for project in error_projects {
                if !text.is_empty() {
                    text.push('\n');
                }
                let last_failed = project
                    .all_build_details
                    .as_ref()
                    .and_then(|details| details.last_failed_build.as_ref());
                if last_failed.is_none() {
                    log::warn!(
                        "No details for the last failed build of project in error: {}",
                        project.url
                    );
                }
                format_project_details(&project.name, last_failed, &mut text);
            }
            backend.show_notification("Build failed", &text, NotificationKind::Error)?;
            return Ok(());
        }

        if !regressing_projects.is_empty() {
            let mut text = String::new();
            for project in regressing_projects {
                if !text.is_empty() {
                    text.push('\n');
                }
                let last_completed = project
                    .all_build_details
                    .as_ref()
                    .and_then(|details| details.last_completed_build.as_ref());
                if last_completed.is_none() {
                    log::warn!(
                        "No details for the last failed build of project in error: {}",
                        project.url
                    );
                }
                format_project_details(&project.name, last_completed, &mut text);
            }
            backend.show_notification("Projects regressed", &text, NotificationKind::Warning)?;
        }
        Ok(())
    }

    /// Whether the status of a project URL is acknowledged
    /// (original: TrayNotifier.IsStatusAcknowledged).
    pub fn is_status_acknowledged(&self, project_url: &str) -> bool {
        self.acknowledged_status_by_project.contains_key(project_url)
    }

    /// Acknowledge the current status of a project
    /// (original: TrayNotifier.AcknowledgeStatus).
    pub fn acknowledge_status(&mut self, project_url: &str, status: BuildStatus) {
        self.acknowledged_status_by_project
            .insert(project_url.to_string(), status);
    }

    /// Forget the acknowledged status (original:
    /// TrayNotifier.ClearAcknowledgedStatus).
    pub fn clear_acknowledged_status(&mut self, project_url: &str) {
        self.acknowledged_status_by_project.remove(project_url);
    }
}

/// "name (user1, user2)" (original: FormatProjectDetails).
fn format_project_details(project_name: &str, build: Option<&crate::model::BuildDetails>, text: &mut String) {
    text.push_str(project_name);
    if let Some(build) = build && !build.users.is_empty() {
        let mut users: Vec<&String> = build.users.iter().collect();
        users.sort();
        let joined: Vec<&str> = users.iter().map(|name| name.as_str()).collect();
        text.push_str(&format!(" ({})", joined.join(", ")));
    }
}

/// 127 character cap with an ellipsis (original: SetNotifyIconText).
fn truncate_tooltip(text: &str) -> String {
    if text.chars().count() > MAX_TOOLTIP_LENGTH {
        let mut truncated: String = text.chars().take(MAX_TOOLTIP_LENGTH - 4).collect();
        truncated.push_str(" ...");
        truncated
    } else {
        text.to_string()
    }
}

/// Bundle of tray backend + notifier state behind a single lock.
pub struct TrayHub {
    pub state: TrayNotifierState,
    pub backend: Box<dyn TrayBackend>,
}

impl TrayHub {
    /// True when the backend's tray icon is actually visible to the user.
    pub fn can_show_icon(&self) -> bool {
        self.backend.can_show_icon()
    }

    /// Create the tray (original: Spring object graph + UpdateNotifierStartup).
    ///
    /// A failing backend never prevents startup - the original's `NotifyIcon`
    /// is created the same way and every later call degrades to logging.
    pub fn new() -> Result<Self> {
        let mut backend = create_tray_backend();
        if let Err(e) = backend.initialize() {
            log::warn!("Tray icon unavailable, degrading to log output: {e:#}");
        }
        if let Err(e) = backend.set_context_menu(
            TRAY_MENU_ITEMS
                .iter()
                .map(|&(label, id)| MenuItem {
                    id,
                    label: label.to_string(),
                    enabled: true,
                    checked: false,
                })
                .collect(),
        ) {
            log::warn!("Tray menu unavailable: {e:#}");
        }
        let state = TrayNotifierState::new();
        if let Err(e) = state.startup(&*backend) {
            log::warn!("Tray startup state unavailable: {e:#}");
        }
        Ok(Self { state, backend })
    }

    /// Refresh icon/tooltip/balloons for the current configuration.
    pub fn update(&mut self, config: &Configuration) {
        self.state.update(config, &*self.backend);
    }

    /// Forwards tray clicks/menu selections (original: the `TrayNotifier`
    /// event handlers wired in `TrayNotifier.Initialize`).
    pub fn set_event_sender(&mut self, sender: TrayEventSender) {
        self.backend.set_event_sender(sender);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_tooltips() {
        let long = "x".repeat(200);
        let truncated = truncate_tooltip(&long);
        assert_eq!(truncated.chars().count(), MAX_TOOLTIP_LENGTH);
        assert!(truncated.ends_with(" ..."));

        assert_eq!(truncate_tooltip("short"), "short");
    }

    #[test]
    fn status_icon_renders_ball() {
        let status = BuildStatus {
            value: BuildStatusEnum::Failed,
            is_in_progress: false,
            is_stuck: false,
        };
        let size = 32;
        let pixels = status_icon_rgba(&status, size);
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        // corner stays transparent, center is the failed ball color
        assert_eq!(&pixels[0..4], &[0, 0, 0, 0]);
        let center = (((size / 2) * size + size / 2) * 4) as usize;
        let [r, g, b, a] = [
            pixels[center],
            pixels[center + 1],
            pixels[center + 2],
            pixels[center + 3],
        ] else {
            panic!("slice must be 4 bytes");
        };
        assert_eq!([r, g, b, a], [0xe5, 0x39, 0x35, 255]);
    }
}
