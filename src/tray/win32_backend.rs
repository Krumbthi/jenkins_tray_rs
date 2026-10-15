//! Windows tray backend: a hand-rolled `Shell_NotifyIconW` implementation -
//! icon, tooltip, context menu (`TrackPopupMenuEx`) and `NIF_INFO` balloons on
//! a message-only window.
//!
//! This mirrors what WinForms' `NotifyIcon` does for the original C# app
//! (`TrayNotifier`); no crate covers the balloon API together with a
//! bring-your-own callback, so the ~150 lines of shell glue live here.
//!
//! NOTE: verified by `cargo check --target x86_64-pc-windows-msvc` only - the
//! WSL development environment cannot run the icon.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, GetCursorPos, GetWindowLongPtrW, ICONINFO,
    LoadCursorW, PostMessageW, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW,
    TrackPopupMenuEx, WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP,
    WNDCLASSEXW, GWLP_USERDATA, HICON, HMENU, HWND_MESSAGE, IDC_ARROW, MF_CHECKED, MF_GRAYED,
    MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TPM_RETURNCMD,
    WINDOW_EX_STYLE, WINDOW_STYLE,
};

use super::{
    status_icon_rgba, MenuItem, NotificationKind, TrayBackend, TrayEvent, TrayEventSender,
    TRAY_MENU_ITEMS,
};
use crate::model::BuildStatus;

/// Callback message for the tray icon (original: `uCallbackMessage` of
/// `NotifyIcon`).
const WM_TRAYICON: u32 = WM_APP + 1;
/// Identifier of our icon (`NOTIFYICONDATA.uID`).
const TRAY_UID: u32 = 1;
/// Balloon duration (original: `TrayNotifier.BALLOON_TOOLTIP_TIMEOUT = 3000`).
/// Windows 10+ ignores it in favour of the system notification setting.
const BALLOON_TIMEOUT_MS: u32 = 3000;
/// Tooltip length cap (original: `TrayNotifier.MAX_TOOLTIP_LENGTH = 127`).
const TOOLTIP_UNITS: usize = 127;

const CLASS_NAME: PCWSTR = w!("JenkinsTrayTrayIcon");
const WINDOW_NAME: PCWSTR = w!("Jenkins Tray");

/// State shared with the window procedure (raw pointer in `GWLP_USERDATA`).
struct TrayCtx {
    sender: Mutex<Option<TrayEventSender>>,
    menu: Mutex<Option<HMENU>>,
}

pub struct Win32Backend {
    /// Message-only window owning the icon (set by `initialize`).
    hwnd: Option<HWND>,
    /// `Shell_NotifyIconW(NIM_ADD)` succeeded - the icon exists.
    added: AtomicBool,
    /// Current icon, destroyed when replaced (original: `NotifyIcon.Icon`).
    icon: Mutex<Option<HICON>>,
    ctx: Box<TrayCtx>,
}

// SAFETY: every Win32 object here is created and used from the UI thread
// (Slint event loop), `Shell_NotifyIconW` is thread-safe, and the fields
// shared with the window procedure are behind `Mutex`/`AtomicBool`.
unsafe impl Send for Win32Backend {}
unsafe impl Sync for Win32Backend {}

impl Win32Backend {
    pub fn new() -> Self {
        Self {
            hwnd: None,
            added: AtomicBool::new(false),
            icon: Mutex::new(None),
            ctx: Box::new(TrayCtx {
                sender: Mutex::new(None),
                menu: Mutex::new(None),
            }),
        }
    }

    /// Fresh `NOTIFYICONDATAW` addressed at our icon.
    fn new_data(&self) -> Result<NOTIFYICONDATAW> {
        let hwnd = self
            .hwnd
            .context("the tray window was not initialized")?;
        Ok(NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            ..Default::default()
        })
    }

    fn require_icon(&self) -> Result<NOTIFYICONDATAW> {
        if !self.added.load(Ordering::SeqCst) {
            return Err(anyhow!("the tray icon is not available"));
        }
        self.new_data()
    }
}

impl TrayBackend for Win32Backend {
    fn can_show_icon(&self) -> bool {
        // Windows always has a taskbar.
        true
    }
    fn initialize(&mut self) -> Result<()> {
        unsafe {
            let hinstance: HINSTANCE = GetModuleHandleW(None)
                .context("GetModuleHandleW failed")?
                .into();
            // Re-registration after a hot restart is fine: the class already exists.
            let _ = RegisterClassExW(&WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            });

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS_NAME,
                WINDOW_NAME,
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance),
                Some((&*self.ctx as *const TrayCtx).cast()),
            )
            .context("CreateWindowExW failed")?;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*self.ctx as *const TrayCtx as isize);

            let menu = CreatePopupMenu().context("CreatePopupMenu failed")?;
            for &(label, id) in TRAY_MENU_ITEMS {
                let label = wide(label);
                AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(label.as_ptr()))
                    .context("AppendMenuW failed")?;
            }
            *self.ctx.menu.lock().unwrap_or_else(|e| e.into_inner()) = Some(menu);

            let icon = create_icon(&status_icon_rgba(&BuildStatus::unknown(), 32), 32, 32)
                .context("initial tray icon")?;
            let mut data = self.new_data()?;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = WM_TRAYICON;
            data.hIcon = icon;
            copy_units(&mut data.szTip, "Jenkins Tray", TOOLTIP_UNITS);

            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                let _ = DestroyIcon(icon);
                let _ = DestroyWindow(hwnd);
                return Err(anyhow!("Shell_NotifyIconW(NIM_ADD) failed"));
            }
            self.hwnd = Some(hwnd);
            self.added.store(true, Ordering::SeqCst);
            *self.icon.lock().unwrap_or_else(|e| e.into_inner()) = Some(icon);
            log::info!("Tray icon registered (Shell_NotifyIconW)");
        }
        Ok(())
    }

    fn update_icon(&self, status: &BuildStatus) -> Result<()> {
        if !self.added.load(Ordering::SeqCst) {
            return Ok(());
        }
        let icon = unsafe { create_icon(&status_icon_rgba(status, 32), 32, 32) }
            .context("tray icon rendering")?;
        let mut data = self.new_data()?;
        data.uFlags = NIF_ICON;
        data.hIcon = icon;
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            let previous = self
                .icon
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .replace(icon);
            if let Some(previous) = previous {
                let _ = unsafe { DestroyIcon(previous) };
            }
            Ok(())
        } else {
            let _ = unsafe { DestroyIcon(icon) };
            Err(anyhow!("Shell_NotifyIconW(NIM_MODIFY) failed"))
        }
    }

    fn set_tooltip(&self, text: &str) -> Result<()> {
        if !self.added.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut data = self.new_data()?;
        data.uFlags = NIF_TIP;
        copy_units(&mut data.szTip, text, TOOLTIP_UNITS);
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            return Err(anyhow!("Shell_NotifyIconW(NIM_MODIFY, NIF_TIP) failed"));
        }
        Ok(())
    }

    fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()> {
        // original: notifyIcon.ShowBalloonTip(BALLOON_TOOLTIP_TIMEOUT, ...)
        let mut data = self.require_icon()?;
        data.uFlags = NIF_INFO;
        let max_title = data.szInfoTitle.len() - 1;
        let max_info = data.szInfo.len() - 1;
        copy_units(&mut data.szInfoTitle, title, max_title);
        copy_units(&mut data.szInfo, message, max_info);
        data.dwInfoFlags = match kind {
            NotificationKind::Info => NIIF_INFO,
            NotificationKind::Warning => NIIF_WARNING,
            NotificationKind::Error => NIIF_ERROR,
        };
        // union of uTimeout/uVersion - only meaningful with NIM_SETVERSION 0
        data.Anonymous.uTimeout = BALLOON_TIMEOUT_MS;
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            return Err(anyhow!("Shell_NotifyIconW(NIM_MODIFY, NIF_INFO) failed"));
        }
        Ok(())
    }

    fn set_context_menu(&self, items: Vec<MenuItem>) -> Result<()> {
        if !self.added.load(Ordering::SeqCst) {
            return Ok(());
        }
        let menu = unsafe { CreatePopupMenu() }.context("CreatePopupMenu failed")?;
        for item in &items {
            let mut flags = MF_STRING;
            if item.checked {
                flags = flags | MF_CHECKED;
            }
            if !item.enabled {
                flags = flags | MF_GRAYED;
            }
            let label = wide(&item.label);
            if let Err(e) =
                unsafe { AppendMenuW(menu, flags, item.id as usize, PCWSTR(label.as_ptr())) }
            {
                let _ = unsafe { DestroyMenu(menu) };
                return Err(e).context("AppendMenuW failed");
            }
        }
        let previous = self
            .ctx
            .menu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(menu);
        if let Some(previous) = previous {
            let _ = unsafe { DestroyMenu(previous) };
        }
        Ok(())
    }

    fn set_event_sender(&mut self, sender: TrayEventSender) {
        *self.ctx.sender.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
    }

    fn exit(&self) -> Result<()> {
        if !self.added.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(hwnd) = self.hwnd {
            let data = self.new_data()?;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                let _ = DestroyWindow(hwnd);
            }
        }
        if let Some(menu) = self
            .ctx
            .menu
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = unsafe { DestroyMenu(menu) };
        }
        if let Some(icon) = self.icon.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = unsafe { DestroyIcon(icon) };
        }
        Ok(())
    }
}

impl Drop for Win32Backend {
    fn drop(&mut self) {
        let _ = self.exit();
    }
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: this is the window procedure of our message-only window; all it
    // does is translate tray messages into channel sends and menu popups.
    unsafe {
        if msg == WM_TRAYICON {
            // Legacy callback format: lParam = mouse message, wParam = icon id.
            // NOTIFYICON_VERSION_4 would swap them; we never enable it, but both
            // positions are accepted to stay robust.
            let candidates = [lparam.0 as u32, wparam.0 as u32];
            let Some(&kind) = candidates
                .iter()
                .find(|m| matches!(**m, WM_LBUTTONDBLCLK | WM_RBUTTONUP | WM_CONTEXTMENU))
            else {
                // WM_LBUTTONUP & co: the original deactivates the single click.
                return LRESULT(0);
            };
            match kind {
                WM_LBUTTONDBLCLK => {
                    // original: notifyIcon_MouseDoubleClick -> ShowOrFocus
                    send_event(hwnd, TrayEvent::OpenRequested);
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// Reads the shared context pointer back from the window.
///
/// # Safety
/// The pointer stays valid until the window is destroyed; the `TrayCtx`
/// owner (`Win32Backend`) outlives the window.
unsafe fn ctx_of(hwnd: HWND) -> Option<&'static TrayCtx> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const TrayCtx).as_ref() }
}

/// SAFETY: `hwnd` must be our tray window (see [`ctx_of`]).
unsafe fn send_event(hwnd: HWND, event: TrayEvent) {
    unsafe {
        let Some(ctx) = ctx_of(hwnd) else { return };
        if let Ok(sender) = ctx.sender.lock() {
            if let Some(sender) = sender.as_ref() {
                let _ = sender.send(event);
            }
        }
    }
}

/// SAFETY: `hwnd` must be our tray window (see [`ctx_of`]).
unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Some(ctx) = ctx_of(hwnd) else { return };
        let Some(menu) = *ctx.menu.lock().unwrap_or_else(|e| e.into_inner()) else {
            return;
        };
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err() {
            return;
        }
        // MSDN tray-menu ritual: without these two calls the menu does not close
        // when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            (TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD).0,
            point.x,
            point.y,
            hwnd,
            None,
        )
        .0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        if cmd != 0 {
            send_event(hwnd, TrayEvent::Menu(cmd));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Zero-terminated UTF-16 for the `W` APIs.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copies at most `max_units` UTF-16 units plus a terminator into a fixed
/// `NOTIFYICONDATAW` buffer (original: 127-char tooltip truncation).
fn copy_units(dest: &mut [u16], text: &str, max_units: usize) {
    let mut len = 0;
    for (i, unit) in text.encode_utf16().take(max_units).enumerate() {
        dest[i] = unit;
        len = i + 1;
    }
    dest[len] = 0;
}

/// Builds an `HICON` from straight-alpha RGBA pixels - the vector tray icon
/// that replaces the original `StatusIcons/*.gif` files. The color DIB is
/// premultiplied (what `CreateIconIndirect` expects), the AND mask is all
/// zeros so transparency comes from the alpha channel alone.
unsafe fn create_icon(rgba: &[u8], width: i32, height: i32) -> Result<HICON> {
    // SAFETY: the GDI objects created here are paired with a DeleteObject and
    // the DIB section buffer is fully written before it is used.
    unsafe {
        let mut bgra = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            let alpha = px[3] as u32;
            let r = (px[0] as u32 * alpha / 255) as u8;
            let g = (px[1] as u32 * alpha / 255) as u8;
            let b = (px[2] as u32 * alpha / 255) as u8;
            bgra.extend_from_slice(&[b, g, r, px[3]]);
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .context("CreateDIBSection failed")?;
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());

        let stride = (((width + 31) / 32) * 4) as usize;
        let mask_bits = vec![0u8; stride * height as usize];
        let mask = CreateBitmap(width, height, 1, 1, Some(mask_bits.as_ptr().cast()));
        if mask.is_invalid() {
            let _ = DeleteObject(color.into());
            return Err(anyhow!("CreateBitmap failed"));
        }

        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: TRUE,
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        })
        .context("CreateIconIndirect failed")?;
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        Ok(icon)
    }
}
