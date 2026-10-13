The origin of this project is https://github.com/zionyx/jenkins-tray. This project is a Rust port of the original C# project.
The user interface is build with Slint [Slint](https://slint.rs/).

## About

This template helps you get started developing a Rust application with Slint as toolkit
for the user interface. It demonstrates the integration between the `.slint` UI markup and
Rust code, how to react to callbacks, get and set properties, and use basic widgets.

## Usage

1. Install Rust by following its [getting-started guide](https://www.rust-lang.org/learn/get-started).
   Once this is done, you should have the `rustc` compiler and the `cargo` build system installed in your `PATH`.
2. Clone this repository into your workspace.
3. Change the working directory and build the application:
    ```
    cd jenkins_tray_rs    
    ```
4. Build with `cargo`:
    ```
    cargo build 
    ```
5. Run the application binary:
    ```
    cargo run
    ```

We recommend using an IDE for development, along with our [LSP-based IDE integration for `.slint` files](https://github.com/slint-ui/slint/blob/master/tools/lsp/README.md). You can also load this project directly in [Visual Studio Code](https://code.visualstudio.com) and install our [Slint extension](https://marketplace.visualstudio.com/items?itemName=Slint.slint).

## Logging

The application logs to stderr via [`env_logger`](https://docs.rs/env_logger). By default
(that is, without the `RUST_LOG` environment variable set) only **error messages** are shown;
routine operational messages (refreshes, tray fallbacks, skipped update checks, ...) stay
silent.

Set `RUST_LOG` to increase verbosity, most usefully `info`:

* Linux / WSL:
    ```
    RUST_LOG=info cargo run
    # or for a built binary:
    RUST_LOG=info ./target/debug/jenkins_tray
    ```
* Windows (PowerShell):
    ```
    $env:RUST_LOG = "info"; cargo run
    ```
* Windows (cmd):
    ```
    set RUST_LOG=info && cargo run
    ```

Levels at a glance:

* `warn` – degradations only (e.g. no tray host available under WSLg, clipboard fallbacks).
* `info` – the full operational log, including startup and the update-feed lines, e.g.
  `Update checks disabled: this port has no update feed yet` at start-up and
  `Update check from User skipped: this port has no update feed yet` when the toolbar's
  *Check for updates* action is used. This port ships without an update feed, see
  `UPDATE_FEED_AVAILABLE` in `src/app_update.rs`.
* `debug` / `trace` – additionally enable framework-internal messages (Slint, reqwest, ...).

Note: release builds for Windows are built with `windows_subsystem = "windows"`, so no
console is attached when the `.exe` is double-clicked and stderr is discarded. Start it from
a terminal (or redirect: `jenkins_tray.exe 2>jenkins_tray.log`) to capture the log.

