//! Single-instance lock and "show the running instance" signalling.
//!
//! New feature (not in the original C# application, which happily starts
//! several times): the first instance holds an exclusive lock file and
//! listens on a loopback TCP port; a second launch asks the running
//! instance to show its window and then exits.
//!
//! This is the way back in when no tray icon exists - e.g. under WSLg,
//! where the session has no StatusNotifierWatcher - so that "closing the
//! window hides it to the tray" (like the `project_tracker` client) can
//! never strand the user: start the application again and its window
//! comes back. On Windows the tray icon reopens the window as well.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use fs2::FileExt;

/// Loopback port the running instance listens on for show requests.
/// Arbitrary but stable; a bind failure only disables the relaunch signal
/// (logged), the single-instance lock itself still works.
const SHOW_PORT: u16 = 39265;

/// The message a second launch sends to the running instance.
const SHOW_COMMAND: &str = "show";

/// Holds the single-instance file lock; the lock is released when the
/// held file is dropped (its file descriptor is closed). The file itself
/// stays on disk - only the advisory lock matters, and the OS releases it
/// with the process, so a crash never leaves a stale lock behind.
pub enum SingleInstance {
    /// This process owns the lock (the file is only kept alive: dropping it
    /// closes the descriptor, which releases the lock).
    Held(#[allow(dead_code)] std::fs::File),
    /// The lock file could not be opened: run without a lock, like the
    /// original application, which allows several instances.
    Unlocked,
}

impl SingleInstance {
    /// Tries to become the only running instance.
    ///
    /// Returns `None` when another instance already holds the lock.
    pub fn acquire() -> Option<Self> {
        let path = lock_file_path();
        match std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false) // the file is only a lock handle; no content
            .open(&path)
        {
            Ok(file) => {
                if file.try_lock_exclusive().is_err() {
                    return None;
                }
                Some(Self::Held(file))
            }
            Err(e) => {
                log::warn!(
                    "Could not open the single-instance lock file {}: {e} - \
                     running without the lock",
                    path.display()
                );
                Some(Self::Unlocked)
            }
        }
    }
}

/// Asks the already-running instance to show its window; returns whether
/// an instance answered.
pub fn notify_running_instance() -> bool {
    match TcpStream::connect((Ipv4Addr::LOCALHOST, SHOW_PORT)) {
        Ok(mut stream) => {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            stream.write_all(SHOW_COMMAND.as_bytes()).is_ok()
        }
        Err(e) => {
            log::debug!("No running instance answered on port {SHOW_PORT}: {e}");
            false
        }
    }
}

/// Serves show requests from further launches on a background thread.
///
/// `on_show` is invoked (on that thread) for every request; the caller
/// marshals onto the UI thread.
pub fn start_show_listener(on_show: impl Fn() + Send + Sync + 'static) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, SHOW_PORT)) {
            Ok(listener) => listener,
            Err(e) => {
                log::warn!(
                    "Could not listen on port {SHOW_PORT} - relaunching the \
                     application will not re-show this window: {e}"
                );
                return;
            }
        };
        let on_show = std::sync::Arc::new(on_show);
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut command = String::new();
            let requested = BufReader::new(&mut stream)
                .read_line(&mut command)
                .is_ok_and(|read| read > 0 && command.trim() == SHOW_COMMAND);
            if requested {
                on_show();
                let _ = stream.write_all(b"ok");
            }
        }
    });
}

/// Lock file location - the platform temp directory, so it gets cleaned on
/// reboot.
fn lock_file_path() -> PathBuf {
    std::env::temp_dir().join("jenkins_tray.lock")
}
