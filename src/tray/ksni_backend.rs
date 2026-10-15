//! Linux tray backend: a StatusNotifierItem over D-Bus ([`ksni`]) for the
//! icon, tooltip, menu and clicks, and the freedesktop Notifications bus
//! ([`zbus`]) for balloons.
//!
//! Original: WinForms `NotifyIcon` (`Shell_NotifyIconW`). Windows always has
//! a taskbar to show an icon; on Linux a tray host
//! (`org.kde.StatusNotifierWatcher`) has to be running - without one (e.g.
//! under WSLg) registration is a soft error, every call is a no-op and
//! main.rs quits on window close instead of hiding to an invisible icon.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::model::BuildStatus;
use super::{
    MenuItem, NotificationKind, TrayBackend, TrayEvent, TrayEventSender, TRAY_MENU_ITEMS,
    status_icon_rgba,
};

/// Icon sizes handed to the host (SNI hosts scale as needed).
const ICON_SIZES: [u32; 2] = [24, 48];

pub struct KsniBackend {
    /// `None` when the session bus is missing (spawn failed); the backend
    /// then no-ops.
    handle: Option<ksni::blocking::Handle<SniTray>>,
}

impl KsniBackend {
    pub fn new() -> Self {
        Self { handle: None }
    }
}

impl TrayBackend for KsniBackend {
    fn initialize(&mut self) -> Result<()> {
        use ksni::blocking::TrayMethods;

        // assume_sni_available(true): a missing tray host is a soft error
        // routed to `watcher_offline` instead of failing the spawn, so the
        // icon appears whenever a host shows up later.
        let handle = SniTray::new()
            .assume_sni_available(true)
            .spawn()
            .context("could not register the StatusNotifierItem")?;
        self.handle = Some(handle);
        log::info!("Tray icon registered (StatusNotifierItem)");
        Ok(())
    }

    fn update_icon(&self, status: &BuildStatus) -> Result<()> {
        let Some(handle) = &self.handle else {
            return Ok(());
        };
        let icons = render_icons(status);
        let _ = handle.update(move |tray| tray.icons = icons);
        Ok(())
    }

    fn set_tooltip(&self, text: &str) -> Result<()> {
        let Some(handle) = &self.handle else {
            return Ok(());
        };
        let text = text.to_string();
        let _ = handle.update(move |tray| tray.tooltip = text);
        Ok(())
    }

    fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()> {
        // original: notifyIcon.ShowBalloonTip(BALLOON_TOOLTIP_TIMEOUT = 3000)
        if let Err(e) = notify(title, message, kind) {
            // Degrade: no notification daemon on the session bus (WSLg,
            // minimal desktops) - the original always has one on Windows.
            log::warn!("Balloon notification unavailable ({e}); original: ShowBalloonTip");
        }
        Ok(())
    }

    fn set_context_menu(&self, items: Vec<MenuItem>) -> Result<()> {
        if let Some(handle) = &self.handle {
            let _ = handle.update(move |tray| tray.menu_items = items);
        }
        Ok(())
    }

    fn set_event_sender(&mut self, sender: TrayEventSender) {
        if let Some(handle) = &self.handle {
            let _ = handle.update(move |tray| tray.sender = Some(sender));
        } else {
            log::debug!("Tray events disabled (no tray backend)");
        }
    }

    fn exit(&self) -> Result<()> {
        if let Some(handle) = &self.handle {
            handle.shutdown().wait();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// StatusNotifierItem
// ---------------------------------------------------------------------------

struct SniTray {
    /// Rendered icons in the SNI ARGB wire format (original:
    /// `notifyIcon.Icon = iconsByKey[key]`).
    icons: Vec<ksni::Icon>,
    /// Hover text (original: `SetNotifyIconText`).
    tooltip: String,
    /// Context menu entries (original: `NotifyIcon.ContextMenuStrip`).
    menu_items: Vec<MenuItem>,
    sender: Option<TrayEventSender>,
}

impl SniTray {
    fn new() -> Self {
        Self {
            icons: render_icons(&BuildStatus::unknown()),
            tooltip: "JenkinsTray".into(), // original: UpdateNotifierStartup
            menu_items: TRAY_MENU_ITEMS
                .iter()
                .map(|&(label, id)| MenuItem {
                    id,
                    label: label.to_string(),
                    enabled: true,
                    checked: false,
                })
                .collect(),
            sender: None,
        }
    }
}

impl ksni::Tray for SniTray {
    fn id(&self) -> String {
        "jenkins_tray".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn status(&self) -> ksni::Status {
        // Passive would let the host hide the icon.
        ksni::Status::Active
    }

    fn title(&self) -> String {
        "Jenkins Tray".into()
    }

    fn icon_name(&self) -> String {
        // Render our own pixmap instead of looking up a theme icon.
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        // The original sets a single 127-char hover text; SNI splits it into
        // title + description, so the status text goes into the description.
        ksni::ToolTip {
            description: self.tooltip.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        // The original opens the window on double click (single click is
        // deactivated in TrayNotifier); SNI only has this one `activate`
        // signal, so any click opens the window.
        if let Some(sender) = &self.sender {
            let _ = sender.send(TrayEvent::OpenRequested);
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.menu_items
            .iter()
            .map(|item| {
                let id = item.id;
                ksni::menu::MenuItem::Standard(ksni::menu::StandardItem {
                    label: item.label.clone(),
                    enabled: item.enabled,
                    activate: Box::new(move |tray: &mut Self| {
                        if let Some(sender) = &tray.sender {
                            let _ = sender.send(TrayEvent::Menu(id));
                        }
                    }),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        log::warn!(
            "Tray host (StatusNotifierWatcher) offline: {reason:?} \
             - the original keeps its icon because Windows always has a taskbar"
        );
        // Keep running: the item registers again when a host returns.
        true
    }

    fn watcher_online(&self) {
        log::info!("Tray host (StatusNotifierWatcher) online");
    }
}

/// Renders the status ball at the advertised sizes (ARGB, network byte order
/// per the StatusNotifierItem spec).
fn render_icons(status: &BuildStatus) -> Vec<ksni::Icon> {
    ICON_SIZES
        .iter()
        .map(|&size| {
            let rgba = status_icon_rgba(status, size);
            let data = rgba
                .as_chunks::<4>().0.iter()
                .flat_map(|p| [p[3], p[0], p[1], p[2]]) // RGBA -> ARGB
                .collect();
            ksni::Icon {
                width: size as i32,
                height: size as i32,
                data,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// D-Bus helpers
// ---------------------------------------------------------------------------

/// `org.freedesktop.Notifications.Notify` (original: `ShowBalloonTip`).
fn notify(title: &str, message: &str, kind: NotificationKind) -> Result<()> {
    let connection =
        zbus::blocking::Connection::session().context("no D-Bus session bus")?;
    let urgency: u8 = match kind {
        NotificationKind::Info => 1,
        NotificationKind::Warning => 1,
        NotificationKind::Error => 2,
    };
    let mut hints = HashMap::new();
    hints.insert("urgency", zbus::zvariant::Value::from(urgency));
    let actions: Vec<String> = Vec::new();
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "jenkins_tray",
                0u32, // replaces_id
                "",   // app_icon
                title,
                message,
                actions,
                hints,
                3000i32, // original: BALLOON_TOOLTIP_TIMEOUT
            ),
        )
        .context("no notification daemon")?;
    Ok(())
}
