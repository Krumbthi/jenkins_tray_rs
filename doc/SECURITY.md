# Recommended architecture: Rust + Slint + Jenkins

For a personal desktop application targeting Windows, macOS, and Linux, I'd use the OS credential manager through Rust's `keyring` crate, `reqwest` for HTTPS, and a small authentication/service layer between Slint and the network.

One important correction to the original requirements: even if you currently want username and password authentication, Jenkins recommends using a username and API token for scripted clients rather than a real account password. Basic authentication supports both, but API tokens reduce the risk of exposing or reusing your actual password;
[image](https://www.google.com/s2/favicons?domain=https://www.jenkins.io\&sz=32)


You can retain username/password support if your Jenkins installation requires it, while designing the application so switching to API tokens later is straightforward.

## 1. Architecture

Slint UI

Jenkins URL, username, password/token, connection status

Rust application services

Credential service · Jenkins client · error handling

OS keychain

Stores the secret

Jenkins REST API

HTTPS + Basic Auth

Keep the Jenkins URL and non-secret preferences in a regular configuration file. Store the password or API token in the OS credential manager. Rust should retrieve the secret only when needed, and Slint should never need to retain it after saving.

## 2. Crates to use

| Crate                   | Purpose                                   |
| ----------------------- | ----------------------------------------- |
| `slint`                 | Cross-platform desktop UI                 |
| `keyring`               | Secure OS credential storage              |
| `reqwest`               | Jenkins HTTP requests with TLS            |
| `tokio`                 | Async network operations                  |
| `serde` + `toml`        | Persist non-secret settings               |
| `anyhow` or `thiserror` | Application error handling                |
| `zeroize`               | Best-effort clearing of sensitive buffers |

For a new implementation, check the current `keyring` documentation before choosing features: version 4 has a different feature model from version 3. The library's default `v1` API is designed for simple cross-platform secret storage. [image](https://www.google.com/s2/favicons?domain=https://docs.rs\&sz=32)

## 3. Configure `Cargo.toml`

For a new project using `keyring` 4.2.x, start with the `v1` API:

```
[dependencies]
slint = "1"
keyring = { version = "4.2", features = ["v1"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
anyhow = "1"
zeroize = "1"
```

The version constraints are examples, not a guarantee of compatibility with every existing Slint project. Check your project's Rust MSRV and dependency tree, and use compatible current releases before locking dependencies.

The `keyring` v1 API selects the platform's native store: macOS Keychain Services, Windows Credential Manager, and Linux Secret Service. [image](https://www.google.com/s2/favicons?domain=https://docs.rs\&sz=32)


Linux caveat: Secret Service usually requires a running desktop-session secret service, such as GNOME Keyring or KWallet, with D-Bus available. Test your app on a clean Linux installation; don't silently fall back to plaintext files if the service is unavailable.

## 4. Implement a credential store

For personal use, I'd use one credential entry per Jenkins server. A stable identifier derived from the server URL prevents credentials for two servers from accidentally sharing the same entry.

For a first version, you can use the normalized Jenkins URL as the keyring username. If you later support multiple Jenkins accounts per server, include the account name in the identifier.

```
use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE: &str = "com.example.my-jenkins-app";

pub struct JenkinsCredentials {
    pub username: String,
    pub password: String,
}

pub fn save_credentials(
    server_id: &str,
    credentials: &JenkinsCredentials,
) -> Result<()> {
    let entry = Entry::new(SERVICE, server_id)
        .context("Could not open the OS credential store")?;

    // Store only the secret here.
    entry
        .set_password(&credentials.password)
        .context("Could not save the Jenkins secret")?;

    Ok(())
}

pub fn load_password(server_id: &str) -> Result<String> {
    let entry = Entry::new(SERVICE, server_id)
        .context("Could not open the OS credential store")?;

    entry
        .get_password()
        .context("Could not retrieve the Jenkins secret")
}

pub fn delete_credentials(server_id: &str) -> Result<()> {
    let entry = Entry::new(SERVICE, server_id)
        .context("Could not open the OS credential store")?;

    entry
        .delete_credential()
        .context("Could not delete the Jenkins secret")
}
```

This uses the documented `Entry` API in `keyring` 4.2.x. [image](https://www.google.com/s2/favicons?domain=https://docs.rs\&sz=32)

The username can be stored in your ordinary configuration file, or stored alongside the secret in a serialized credential record. For this personal application, keeping the username in configuration and the password in the keyring is simpler.

A few implementation details matter:

- Normalize and validate the Jenkins URL before deriving `server_id`.
- Do not use a raw URL containing embedded credentials as the identifier.
- Save the new credential before deleting an old one when changing server/account associations.
- Treat a missing entry differently from a locked or unavailable credential store.
- Do not log errors containing credentials or HTTP request headers.

## 5. Authenticate to Jenkins with `reqwest`

For compatibility with your stated username/password requirement, use HTTP Basic authentication over HTTPS. The `reqwest` `basic_auth` method constructs the required authentication header.

```
use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

pub async fn test_jenkins_connection(
    client: &Client,
    base_url: &str,
    username: &str,
    password: &str,
) -> Result<()> {
    let url = format!(
        "{}/api/json",
        base_url.trim_end_matches('/')
    );

    let response = client
        .get(&url)
        .basic_auth(username, Some(password))
        .send()
        .await
        .context("Jenkins request failed")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Jenkins returned HTTP status {}",
            response.status()
        );
    }

    Ok(())
}

pub fn create_http_client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .https_only(true)
        .build()?)
}
```

This example intentionally uses `https_only(true)`. If your Jenkins server is available only over plain HTTP, fix the deployment or create an explicitly restricted development-only configuration rather than silently weakening production security.

For a self-signed certificate, configure trust for the correct certificate or private CA. Do not disable certificate verification.

Also note that Jenkins may return `403 Forbidden` for failed authentication or insufficient permissions instead of the `401` you might expect. Diagnose authentication and authorization failures without printing secrets. [image](https://www.google.com/s2/favicons?domain=https://www.jenkins.io\&sz=32)

### CSRF protection

For read-only API calls, the example above is a reasonable starting point. If you trigger builds or perform other state-changing operations, check your Jenkins CSRF configuration and use the crumb mechanism where required. Jenkins API tokens have special CSRF handling, another reason to prefer them when possible. [image](https://www.google.com/s2/favicons?domain=https://www.jenkins.io\&sz=32)

## 6. Integrate with Slint without blocking the UI

Slint should handle input and presentation. Rust should handle keyring operations and HTTP requests.

A good flow is:

1. The user enters the Jenkins URL and credentials.
2. A Slint callback passes the values into a Rust service.
3. The service stores the secret and updates the connection state.
4. Network requests run asynchronously, not on the UI thread.
5. The UI receives only a success/failure result and any data it needs to display.

Avoid keeping the password in a global Slint property for the lifetime of the application. If the UI framework requires a string property for a password input, clear it after saving or cancelling the form. Clear the password field on authentication failure if appropriate.

For `zeroize`, remember that it is only a best-effort mitigation: ordinary Rust `String` values and copies created by HTTP libraries are not automatically guaranteed to be erased from memory. Avoid unnecessary copies and keep credential lifetimes short.

## 7. Platform-specific considerations

| Platform | Credential store   | Main caveat                                                                                                                                |
| -------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Windows  | Credential Manager | Test install and execution under the actual user account; credentials are associated with that account and its Windows security context.   |
| macOS    | Keychain           | Users may see access prompts depending on Keychain state and app identity. Test the signed and packaged application, not only `cargo run`. |
| Linux    | Secret Service     | The desktop keyring service and D-Bus session must be available. Headless sessions may not have them.                                      |

For all three platforms, the OS credential manager protects secrets at rest, but it does not protect against malware or a compromised process running as the logged-in user.

## 8. Final recommendation

I'd implement the first version with these choices:

- `keyring` for storing the password.
- A TOML file for the Jenkins URL, username, and other non-secret preferences.
- `reqwest` with TLS for API calls.
- A Rust service layer that owns credential retrieval and authentication.
- Slint for the UI only.
- No plaintext fallback when secure storage is unavailable.

Then, unless you have a specific reason to require a real password, add support for Jenkins API tokens as the preferred authentication option. Jenkins explicitly recommends username plus API token for scripted clients. [image](https://www.google.com/s2/favicons?domain=https://www.jenkins.io\&sz=32)

This keeps the architecture simple for personal use, portable across your target platforms, and easy to evolve later.
