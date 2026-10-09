# Codebase Index: jenkins_tray

> Generated: 2026-10-09 13:52:36 UTC | Files: 13 | Lines: 5346
> Languages: Markdown (3), Rust (9), TOML (1)

## Directory Structure

```
jenkins_tray/
  Cargo.toml
  INDEX.md
  PORT_INSTRUCTIONS.md
  README.md
  build.rs
  src/
    config/
      configuration.rs
    config.rs
    jenkins_client.rs
    main.rs
    model.rs
    notifications.rs
    projects_updater.rs
    tray.rs
```

---

## Public API Surface

**Cargo.toml**
- `[package]`
- `[dependencies]`
- `[build-dependencies]`
- `[profile.release]`

**INDEX.md**
- `# Codebase Index: jenkins_tray`

**PORT_INSTRUCTIONS.md**
- `### I'd structure the Rust port like this`
- `## The model should be different from what we were doing before`
- `## And I'd simplify the Slint model`
- `## The main window should reproduce the original functionality`
- `## The tray is particularly important`
- `## One thing I would *not* port literally`

**README.md**
- `# Slint Rust Template`

**src/config/configuration.rs**
- `pub const CONFIGURATION_FILE: &str = "jenkins_tray.json"`
- `pub const LEGACY_CONFIGURATION_FILE: &str = "jenkins.configuration"`
- `pub const JENKINS_TRAY_DIRECTORY: &str = "Jenkins Tray"`
- `pub fn config_file_path() -> anyhow::Result<std::path::PathBuf>`
- `pub fn load_configuration() -> anyhow::Result<crate::model::Configuration>`
- `pub fn save_configuration(config: &crate::model::Configuration) -> anyhow::Result<()>`

**src/config.rs**
- `pub mod configuration`
- `pub struct ConfigurationService`

**src/jenkins_client.rs**
- `pub const BUILD_DETAILS_FILTER: &str = "?tree=number,fullDisplayName,timestamp,estimatedDuration,duration,result,userNodes,culprits[fullName],builtOn,url,actions[assignedBy,claimDate,claimed,claimedBy,reason]"`
- `pub struct Clients`
- `pub fn build_client(ignore_untrusted_certificate: bool) -> Result<reqwest::Client>`
- `pub struct Credentials`
- `pub fn concat_urls(base: &str, fragments: &[&str]) -> String`
- `pub struct XmlCache`
- `pub type SharedCache = Arc<Mutex<XmlCache>>`
- `pub fn new_cache() -> SharedCache`
- `pub fn recycle_cache(cache: &SharedCache)`
- `pub fn remove_cache_entry(cache: &SharedCache, url: &str)`
- `pub struct JobEntry`
- `pub fn parse_job_list(xml: &str) -> Result<Vec<JobEntry>>`
- `pub async fn load_projects(client: &reqwest::Client, server: &Server) -> Result<Vec<Project>>`
- `pub struct JobState`
- `pub fn parse_job_state(xml: &str) -> Result<JobState>`
- `pub async fn update_project( client: &reqwest::Client, server: &Server, project: &Project, cache: &SharedCache, ) -> Result<ProjectState>`
- `pub fn parse_build_details(xml: &str) -> Result<BuildDetails>`
- `pub async fn trigger_build( client: &reqwest::Client, server: &Server, project: &Project, ) -> Result<()>`
- `pub async fn stop_build( client: &reqwest::Client, server: &Server, project: &Project, ) -> Result<()>`
- `pub async fn remove_from_queue( client: &reqwest::Client, server: &Server, queue_id: u64, ) -> Result<()>`
- `pub async fn claim_build( server: &Server, project: &Project, build_number: u32, reason: &str, sticky: bool, cache: &SharedCache, ) -> Result<()>`

**src/model.rs**
- `pub struct Server`
- `pub struct Project`
- `pub struct BuildStatus`
- `pub enum BuildStatusEnum`
- `pub struct QueueItem`
- `pub struct BuildDetails`
- `pub struct AllBuildDetails`
- `pub struct ClaimDetails`
- `pub enum BuildCauseEnum`
- `pub struct BuildCause`
- `pub struct BuildCauses`
- `pub struct ProjectState`
- `pub struct Configuration`
- `pub struct NotificationSettings`
- `pub struct GeneralSettings`

**src/notifications.rs**
- `pub trait TrayNotifier: Send + Sync`
- `pub struct AllServersStatus`
- `pub struct NotificationService`

**src/projects_updater.rs**
- `pub struct UpdaterConfig`
- `pub type ProjectsUpdatedHandler = Arc<dyn Fn() + Send + Sync>`
- `pub struct ProjectsUpdateService`

**src/tray.rs**
- `pub const MAX_TOOLTIP_LENGTH: usize = 127`
- `pub enum NotificationKind`
- `pub trait TrayBackend: Send + Sync`
- `pub struct MenuItem`
- `pub const TRAY_MENU_ITEMS: &[(&str, u32)] = &[ ("Open", 1), ("Settings", 2), ("Refresh", 3), ("About", 4), ("Exit", 5), ]`
- `pub struct StubTrayBackend`
- `pub fn create_tray_backend() -> Result<Box<dyn TrayBackend>>`
- `pub struct TrayNotifierState`
- `pub struct TrayHub`

---

## Cargo.toml

**Language:** TOML | **Size:** 683 B | **Lines:** 27

**Imports:**
- `slint`
- `tokio`
- `anyhow`
- `log`
- `serde`
- `reqwest`
- `serde_json`
- `env_logger`
- `chrono`
- `roxmltree`
- *... and 1 more imports*

**Declarations:**

---

## INDEX.md

**Language:** Markdown | **Size:** 17.6 KB | **Lines:** 614

**Declarations:**

---

## PORT_INSTRUCTIONS.md

**Language:** Markdown | **Size:** 7.8 KB | **Lines:** 272

**Declarations:**

---

## README.md

**Language:** Markdown | **Size:** 1.7 KB | **Lines:** 38

**Declarations:**

---

## build.rs

**Language:** Rust | **Size:** 91 B | **Lines:** 3

**Declarations:**

`fn main()`

---

## src/config/configuration.rs

**Language:** Rust | **Size:** 6.8 KB | **Lines:** 177

**Declarations:**

`fn config_file_path_for(file_name: &str) -> anyhow::Result<std::path::PathBuf>`

`fn app_data_dir() -> Option<std::path::PathBuf>`

`fn env_dir(name: &str) -> Option<std::path::PathBuf>`

`fn normalize(mut config: crate::model::Configuration) -> crate::model::Configuration`

`mod tests`

---

## src/config.rs

**Language:** Rust | **Size:** 11.4 KB | **Lines:** 305

**Imports:**
- `std::sync::RwLock`
- `crate::model::{
    Configuration, GeneralSettings, NotificationSettings, Project, ProjectState, QueueItem, Server,
}`

**Declarations:**

**`impl ConfigurationService`**
  `pub fn new() -> anyhow::Result<Self>`

  `pub fn get_config(&self) -> anyhow::Result<Configuration>`

  `pub fn save(&self) -> anyhow::Result<()>`

  `fn mutate(&self, f: impl FnOnce(&mut Configuration)) -> anyhow::Result<()>`

  `pub fn add_server( &self, url: &str, display_name: &str, username: Option<&str>, password: Option<&str>, ignore_untrusted_certificate: bool, ) -> anyhow::Result<Server>`

  `pub fn update_server( &self, old_url: &str, url: &str, display_name: &str, username: Option<&str>, password: Option<&str>, ignore_untrusted_certificate: bool, ) -> anyhow::Result<()>`

  `pub fn remove_server(&self, server: &Server) -> anyhow::Result<()>`

  `pub fn add_project(&self, project: &Project) -> anyhow::Result<()>`

  `pub fn add_projects(&self, projects: &[Project]) -> anyhow::Result<()>`

  `fn insert_project(config: &mut Configuration, project: &Project)`

  `pub fn remove_project(&self, project: &Project) -> anyhow::Result<()>`

  `pub fn remove_projects(&self, projects: &[Project]) -> anyhow::Result<()>`

  `fn extract_project(config: &mut Configuration, project: &Project)`

  `pub fn set_project_authentication( &self, server_url: &str, project_url: &str, token: Option<&str>, cause: Option<&str>, ) -> anyhow::Result<()>`

  `pub fn set_project_acknowledged( &self, server_url: &str, project_url: &str, acknowledged: bool, ) -> anyhow::Result<()>`

  `fn find_project_mut<'a>( config: &'a mut Configuration, server_url: &str, project_url: &str, ) -> Option<&'a mut Project>`

  `pub fn apply_project_states( &self, states: &[(String, ProjectState)], ) -> anyhow::Result<()>`

  `pub fn get_general_settings(&self) -> anyhow::Result<GeneralSettings>`

  `pub fn set_general_settings(&self, settings: GeneralSettings) -> anyhow::Result<()>`

  `pub fn get_notification_settings(&self) -> anyhow::Result<NotificationSettings>`

  `pub fn set_notification_settings(&self, settings: NotificationSettings) -> anyhow::Result<()>`

  `pub fn get_refresh_interval(&self) -> anyhow::Result<u64>`


---

## src/jenkins_client.rs

**Language:** Rust | **Size:** 31.5 KB | **Lines:** 846

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `std::sync::{Arc, Mutex}`
- `std::time::Duration`
- `anyhow::{anyhow, Context, Result}`
- `roxmltree::{Document, Node}`
- `crate::model::{
    AllBuildDetails, BuildCauses, BuildCause, BuildCauseEnum, BuildDetails, BuildStatus,
    ClaimDetails, Project, ProjectState, Server,
}`

**Declarations:**

`const JOB_LIST_TREE: &str = "api/xml?tree=jobs[name,url,color]"`

**`impl Clients`**
  `pub fn new() -> Result<Self>`

  `pub fn for_server(&self, server: &Server) -> &reqwest::Client`


**`impl From<&Server> for Credentials`**
  `fn from(server: &Server) -> Self`


`fn authenticated(req: reqwest::RequestBuilder, credentials: &Credentials) -> reqwest::RequestBuilder`

`async fn get_text( client: &reqwest::Client, credentials: &Credentials, url: &str, ) -> Result<String>`

`async fn upload_string( credentials: &Credentials, request: reqwest::RequestBuilder, ) -> Result<String>`

`async fn download( client: &reqwest::Client, credentials: &Credentials, url: &str, cache: Option<&SharedCache>, ) -> Result<String>`

`fn element_children<'doc, 'input>(node: Node<'doc, 'input>) -> impl Iterator<Item = Node<'doc, 'input>>`

`fn find_child<'doc, 'input>(node: Node<'doc, 'input>, name: &str) -> Option<Node<'doc, 'input>>`

`fn child_text<'doc, 'input>(node: Node<'doc, 'input>, name: &str) -> Option<&'doc str>`

`fn child_bool<'doc, 'input>(node: Node<'doc, 'input>, name: &str) -> Option<bool>`

`fn extract_user_name(full_name: &str) -> String`

`fn collect_projects<'a>( client: &'a reqwest::Client, credentials: &'a Credentials, list_url: &'a str, server: &'a Server, projects: &'a mut Vec<Project>, ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>`

`async fn get_build_details( client: &reqwest::Client, credentials: &Credentials, build_url: &str, cache: &SharedCache, ) -> Result<BuildDetails>`

`fn fill_in_build_causes(root: Node<'_, '_>) -> BuildCauses`

`fn non_empty(value: Option<&str>) -> Option<&str>`

`fn fill_in_claim_details(root: Node<'_, '_>) -> Option<ClaimDetails>`

`mod tests`

---

## src/main.rs

**Language:** Rust | **Size:** 66.0 KB | **Lines:** 1672

**Imports:**
- `std::cell::RefCell`
- `std::collections::{HashMap, HashSet}`
- `std::rc::Rc`
- `std::sync::{Arc, Mutex, MutexGuard}`
- `anyhow::Result`
- `chrono::{Duration, Local, TimeZone}`
- `slint::{ComponentHandle, ModelRc, VecModel}`
- `config::ConfigurationService`
- `model::{BuildCauseEnum, BuildDetails, BuildStatusEnum, Configuration, Project, Server}`
- `notifications::NotificationService`
- *... and 2 more imports*

**Declarations:**

`mod config`

`mod jenkins_client`

`mod model`

`mod notifications`

`mod projects_updater`

`mod tray`

`struct EditServerContext`
> Fields: `index: Option<usize>`

`struct AppState`
> Fields: `row_keys: Vec<Option<(String, String)>>`, `settings_projects: HashMap<String, Vec<Project>>`, `edit_server: EditServerContext`, `auth_token_target: Option<(String, String)>`, `claim_target: Option<(String, String, u32)>`

`struct Services`
> Fields: `config: Arc<ConfigurationService>`, `notifications: Arc<NotificationService>`, `clients: jenkins_client::Clients`, `updater: Rc<RefCell<ProjectsUpdateService>>`, `state: Arc<Mutex<AppState>>`, `tray: Arc<Mutex<TrayHub>>`, `runtime: tokio::runtime::Handle`

`fn lock_state(state: &Arc<Mutex<AppState>>) -> MutexGuard<'_, AppState>`

`struct Windows`
> Fields: `settings: Rc<SettingsWindow>`, `edit_server: Rc<EditServerWindow>`, `about: Rc<AboutWindow>`, `auth_token: Rc<AuthTokenWindow>`, `claim_build: Rc<ClaimBuildWindow>`

`fn server_display_text(server: &Server) -> String`

`fn project_in<'a>( config: &'a Configuration, server_url: &str, project_url: &str, ) -> Option<(&'a Server, &'a Project)>`

`fn selected_target( ui: &AppWindow, state: &Arc<Mutex<AppState>>, ) -> Option<(String, String)>`

`fn sorted_names(users: &HashSet<String>) -> Vec<String>`

`fn format_users(users: Option<&HashSet<String>>) -> String`

`fn format_build(number: u32, time: Option<i64>) -> String`

`fn capitalize(text: &str) -> String`

`fn format_details(project: &Project) -> String`

`fn format_duration(build: &BuildDetails) -> String`

`fn format_estimated_duration(build: &BuildDetails) -> String`

`fn format_build_details(build: Option<&BuildDetails>) -> String`

`fn project_row(project: &Project, server: &Server, show_display_name: bool) -> ProjectRow`

`fn build_rows(config: &Configuration) -> (Vec<ProjectRow>, Vec<Option<(String, String)>>)`

`fn refresh_rows( ui: &AppWindow, config_service: &ConfigurationService, state: &Arc<Mutex<AppState>>, ) -> Result<()>`

`fn on_configuration_updated(services: &Services, ui: &AppWindow)`

`fn populate_settings(services: &Services, settings: &SettingsWindow) -> Result<()>`

`fn spawn_project_load(services: &Services, settings: &SettingsWindow, server: &Server)`

`fn populate_project_list( config_service: &ConfigurationService, settings: &SettingsWindow, state: &Arc<Mutex<AppState>>, server_index: i32, ) -> Result<()>`

`fn set_sound_path(paths: &mut HashMap<String, String>, key: &str, value: String)`

`fn is_valid_server_url(url: &str) -> bool`

`fn normalize_server_url(url: &str) -> String`

`fn console_page(project: &Project) -> String`

`fn open_url(url: &str)`

`fn wire_callbacks(ui: &AppWindow, services: &Services, windows: &Windows)`

`fn main() -> Result<()>`

---

## src/model.rs

**Language:** Rust | **Size:** 16.6 KB | **Lines:** 436

**Imports:**
- `serde::{Deserialize, Serialize}`
- `std::collections::{HashMap, HashSet}`
- `std::cmp::Ordering`

**Declarations:**

**`impl Project`**
  `pub fn status(&self) -> BuildStatus`

  `pub fn has_build(&self) -> bool`


**`impl BuildStatus`**
  `pub fn unknown() -> Self`

  `pub fn from_jenkins_color(color: &str, stuck: bool) -> Self`

  `pub fn from_jenkins_result(result: &str) -> BuildStatusEnum`

  `pub fn is_worse(&self, than: &BuildStatus) -> bool`

  `pub fn treat_as_failure(&self) -> bool`

  `pub fn key(&self) -> String`


**`impl std::fmt::Display for BuildStatusEnum`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


**`impl Ord for BuildStatusEnum`**
  `fn cmp(&self, other: &Self) -> Ordering`


**`impl PartialOrd for BuildStatusEnum`**
  `fn partial_cmp(&self, other: &Self) -> Option<Ordering>`


**`impl Default for GeneralSettings`**
  `fn default() -> Self`


---

## src/notifications.rs

**Language:** Rust | **Size:** 9.2 KB | **Lines:** 269

**Imports:**
- `std::sync::Mutex`
- `anyhow::Result`
- `crate::model::{BuildStatus, BuildStatusEnum, Configuration, Project, Server}`

**Declarations:**

**`impl Default for AllServersStatus`**
  `fn default() -> Self`


**`impl AllServersStatus`**
  `pub fn new() -> Self`

  `pub fn update(&mut self, servers: &[Server])`

  `fn update_for_project(&mut self, project: &Project)`


**`impl NotificationService`**
  `pub fn new() -> Self`

  `pub fn set_tray_notifier(&mut self, tray: Box<dyn TrayNotifier>)`

  `pub fn execute(&self, config: &Configuration) -> Result<()>`

  `pub fn play_sound(&self, path: &str) -> Result<()>`

  `fn play_file(&self, path: &str)`


`mod tests`

---

## src/projects_updater.rs

**Language:** Rust | **Size:** 8.4 KB | **Lines:** 240

**Imports:**
- `std::sync::Arc`
- `std::time::Duration`
- `tokio::sync::{watch, Notify, Semaphore}`
- `tokio::task::JoinHandle`
- `crate::config::ConfigurationService`
- `crate::jenkins_client::{self, Clients, SharedCache}`
- `crate::model::ProjectState`

**Declarations:**

**`impl Default for UpdaterConfig`**
  `fn default() -> Self`


**`impl ProjectsUpdateService`**
  `pub fn new( config: UpdaterConfig, clients: Clients, config_service: Arc<ConfigurationService>, ) -> Self`

  `pub fn set_on_updated(&mut self, handler: ProjectsUpdatedHandler)`

  `pub fn start(&mut self, runtime: &tokio::runtime::Handle)`

  `pub fn refresh_all(&self)`

  `pub fn cache(&self) -> SharedCache`

  `pub async fn stop(&mut self)`


`struct UpdateLoop`
> Fields: `config: UpdaterConfig`, `clients: Clients`, `config_service: Arc<ConfigurationService>`, `cache: SharedCache`, `wake: Arc<Notify>`, `on_updated: Option<ProjectsUpdatedHandler>`

**`impl UpdateLoop`**
  `fn refresh_interval(&self) -> u64`

  `async fn run_cycle(&self)`


---

## src/tray.rs

**Language:** Rust | **Size:** 16.1 KB | **Lines:** 447

**Imports:**
- `std::collections::HashMap`
- `anyhow::Result`
- `crate::model::{AllBuildDetails, BuildStatus, BuildStatusEnum, Configuration, Project}`
- `crate::notifications::TrayNotifier`

**Declarations:**

**`impl TrayBackend for StubTrayBackend`**
  `fn initialize(&mut self) -> Result<()>`

  `fn update_icon(&self, status: &BuildStatus) -> Result<()>`

  `fn set_tooltip(&self, text: &str) -> Result<()>`

  `fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()>`

  `fn set_context_menu(&self, _items: Vec<MenuItem>) -> Result<()>`

  `fn exit(&self) -> Result<()>`


**`impl TrayNotifier for StubTrayBackend`**
  `fn play_sound(&self, path: &str) -> Result<()>`


**`impl TrayNotifierState`**
  `pub fn new() -> Self`

  `pub fn startup(&self, backend: &dyn TrayBackend) -> Result<()>`

  `pub fn update(&mut self, config: &Configuration, backend: &dyn TrayBackend)`

  `fn do_update(&mut self, config: &Configuration, backend: &dyn TrayBackend) -> Result<()>`

  `fn get_project_status(&mut self, project: &Project) -> BuildStatus`

  `fn is_regressing(&self, project: &Project) -> bool`

  `fn update_tray_tooltip( &mut self, progressing_and_error_projects: &[&Project], total_project_count: usize, backend: &dyn TrayBackend, ) -> Result<()>`

  `fn update_balloon_tip( &mut self, error_projects: &[&Project], regressing_projects: &[&Project], backend: &dyn TrayBackend, ) -> Result<()>`

  `pub fn is_status_acknowledged(&self, project_url: &str) -> bool`

  `pub fn acknowledge_status(&mut self, project_url: &str, status: BuildStatus)`

  `pub fn clear_acknowledged_status(&mut self, project_url: &str)`


`fn format_project_details(project_name: &str, build: Option<&crate::model::BuildDetails>, text: &mut String)`

`fn truncate_tooltip(text: &str) -> String`

**`impl TrayHub`**
  `pub fn new() -> Result<Self>`

  `pub fn update(&mut self, config: &Configuration)`


`mod tests`

