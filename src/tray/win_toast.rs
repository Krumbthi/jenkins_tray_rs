//! Native Windows toast notifications for the unpackaged desktop app.
//!
//! Slint owns the visible tray icon, so the old `Shell_NotifyIconW` balloon
//! path cannot be used: balloons belong to the tray icon instance that created
//! them, and Slint does not expose that handle. WinRT toasts do not need the
//! tray icon; they only need an AppUserModelID (AUMID) that Windows can show in
//! notification settings.
//!
//! On startup the AUMID is registered under
//! `HKCU\Software\Classes\AppUserModelId\<APP_ID>` and written to a per-user
//! Start Menu shortcut. Both are needed for an unpackaged, portable
//! executable: the registry entry gives the toast an identity and display
//! name, and the shortcut gives the shell a concrete launch target for that
//! identity (Microsoft documents the shortcut as a prerequisite). No installer
//! or administrator rights are required.

use anyhow::{Context, Result};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::HSTRING;

/// Identifies this application to the Windows notification platform.
const APP_ID: &str = "Krumbthi.JenkinsTray";
const DISPLAY_NAME: &str = "Jenkins Tray";

/// Register the notification identity, create the Start Menu shortcut that
/// Windows requires for desktop toasts, and set the process AUMID.
///
/// Failures are only logged by the caller: a machine policy may forbid toast
/// notifications, but that must never prevent the application from starting.
pub fn initialize() {
    if let Err(e) = register_app_id() {
        log::warn!("Could not register the notification AppUserModelID: {e:#}");
    }
    if let Err(e) = ensure_start_menu_shortcut() {
        log::warn!("Could not create the notification Start Menu shortcut: {e:#}");
    }

    let app_id = HSTRING::from(APP_ID);
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&app_id) } {
        log::warn!("Could not set the process AppUserModelID: {e}");
    }
}

/// Show a native toast in the bottom-right corner.
pub fn show(title: &str, message: &str) -> Result<()> {
    // `ToastGeneric` is the modern template; two text elements render as the
    // bold title and the body. The sound is disabled because sound
    // notifications are handled separately by `NotificationService`.
    let xml = format!(
        r#"<toast duration="short"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/></toast>"#,
        escape_xml(title),
        escape_xml(message)
    );

    let document = XmlDocument::new().context("could not create the toast XML document")?;
    document
        .LoadXml(&HSTRING::from(xml))
        .context("could not load the toast XML")?;
    let toast = ToastNotification::CreateToastNotification(&document)
        .context("could not create the toast notification")?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
        .context("could not create the toast notifier")?;
    notifier
        .Show(&toast)
        .context("could not show the toast notification")?;
    Ok(())
}

/// Creates/updates the per-user AUMID registration.
fn register_app_id() -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let exe = std::env::current_exe().context("could not resolve the executable path")?;
    let key_path = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(&key_path)?;
    key.set_value("DisplayName", &DISPLAY_NAME)?;
    key.set_value("IconUri", &exe.to_string_lossy().into_owned())?;
    key.set_value("ShowInSettings", &1u32)?;
    Ok(())
}

/// Creates/updates the Start Menu shortcut that carries the AUMID.
///
/// Microsoft documents this shortcut as a prerequisite for toasts from a
/// desktop app: unlike a packaged app, a plain Win32 process has no package
/// identity. The shortcut lives in the per-user Start Menu, so no installer or
/// administrator rights are needed; it is idempotently rewritten on startup so
/// it follows a moved executable.
fn ensure_start_menu_shortcut() -> Result<()> {
    use std::path::PathBuf;

    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::Interface;

    let exe = std::env::current_exe().context("could not resolve the executable path")?;
    let exe = exe.to_string_lossy().into_owned();
    let appdata = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    let shortcut = PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Jenkins Tray.lnk");
    let shortcut = shortcut.to_string_lossy().into_owned();

    // The UI thread may already have initialized COM (Slint/winit); in that
    // case CoInitializeEx reports S_FALSE or RPC_E_CHANGED_MODE. Both are
    // harmless for CoCreateInstance, so the result is deliberately ignored.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .context("could not create the shell link")?;
        let exe_hstring = HSTRING::from(exe.as_str());
        link.SetPath(&exe_hstring)
            .context("could not set the shortcut target")?;
        link.SetDescription(&HSTRING::from(DISPLAY_NAME))
            .context("could not set the shortcut description")?;
        link.SetIconLocation(&exe_hstring, 0)
            .context("could not set the shortcut icon")?;

        let store: IPropertyStore = link
            .cast()
            .context("could not query the shortcut property store")?;
        // `PROPVARIANT::from(&str)` creates a `VT_BSTR` whose memory is freed
        // by the type's `Drop` (via `PropVariantClear`). Do not build a
        // `VT_LPWSTR` variant from an `HSTRING` pointer: the drop would free
        // memory that was not allocated with `CoTaskMemAlloc`.
        let value = PROPVARIANT::from(APP_ID);
        store
            .SetValue(&PKEY_AppUserModel_ID, &value)
            .context("could not set the shortcut AppUserModelID")?;
        store
            .Commit()
            .context("could not commit the shortcut properties")?;

        let persist: IPersistFile = link
            .cast()
            .context("could not query the shortcut file interface")?;
        persist
            .Save(&HSTRING::from(shortcut.as_str()), true)
            .context("could not save the Start Menu shortcut")?;
    }
    Ok(())
}

/// Escapes the five XML predefined entities. Project names come from Jenkins
/// and may contain `&`, `<`, etc.
fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
