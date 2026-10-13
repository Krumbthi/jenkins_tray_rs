# Codebase Index: jenkins_tray

> Generated: 2026-10-12 14:05:12 UTC | Files: 17 | Lines: 8295
> Languages: Markdown (3), Rust (13), TOML (1)

## Directory Structure

```
jenkins_tray/
  Cargo.toml
  INDEX.md
  PORT_INSTRUCTIONS.md
  README.md
  build.rs
  src/
    app_update.rs
    config/
      configuration.rs
    config.rs
    errors.rs
    jenkins_client.rs
    main.rs
    model.rs
    notifications.rs
    projects_updater.rs
    tray/
      ksni_backend.rs
      win32_backend.rs
    tray.rs
```

---

## Public API Surface

**Cargo.toml**
- `[package]`
- `[dependencies]`
- `[target.'cfg(all(unix, not(target_os = "macos")))'.dependencies]`
- `[target.'cfg(windows)'.dependencies]`
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

**src/app_update.rs**
- `pub enum UpdateSource`
- `pub struct ApplicationUpdateService`

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

**src/errors.rs**
- `pub fn root_cause(error: &anyhow::Error) -> String`

**src/jenkins_client.rs**
- `pub const BUILD_DETAILS_FILTER: &str = "?tree=number,fullDisplayName,timestamp,estimatedDuration,duration,result,userNodes,culprits[fullName],builtOn,url,actions[assignedBy,claimDate,claimed,claimedBy,reason]"`
- `pub struct Clients`
- `pub fn build_client(ignore_untrusted_certificate: bool) -> Result<reqwest::Client>`
- `pub struct Credentials`
- `pub fn concat_urls(base: &str, fragments: &[&str]) -> String`
- `pub fn rebase_url(url: &str, server: &Server) -> String`
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
- `pub enum ParamKind`
- `pub struct JobParameter`
- `pub fn parse_build_parameters(json: &str) -> Result<Vec<JobParameter>>`
- `pub async fn get_build_parameters( client: &reqwest::Client, server: &Server, project: &Project, ) -> Result<Vec<JobParameter>>`
- `pub async fn trigger_build_parameters( client: &reqwest::Client, server: &Server, project: &Project, params: &[(String, String)], ) -> Result<()>`
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

**src/tray/ksni_backend.rs**
- `pub struct KsniBackend`

**src/tray/win32_backend.rs**
- `pub struct Win32Backend`

**src/tray.rs**
- `pub const MAX_TOOLTIP_LENGTH: usize = 127`
- `pub enum NotificationKind`
- `pub enum TrayEvent`
- `pub type TrayEventSender = std::sync::mpsc::Sender<TrayEvent>`
- `pub trait TrayBackend: Send + Sync`
- `pub struct MenuItem`
- `pub const TRAY_MENU_ITEMS: &[(&str, u32)] = &[ ("Open", 1), ("Settings", 2), ("Refresh", 3), ("About", 4), ("Exit", 5), ]`
- `pub struct StubTrayBackend`
- `pub fn create_tray_backend() -> Box<dyn TrayBackend>`
- `pub fn status_icon_rgba(status: &BuildStatus, size: u32) -> Vec<u8>`
- `pub struct TrayNotifierState`
- `pub struct TrayHub`

---

## Cargo.toml

**Language:** TOML | **Size:** 1.7 KB | **Lines:** 50

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

**Language:** Markdown | **Size:** 26.8 KB | **Lines:** 946

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

**Language:** Rust | **Size:** 2.0 KB | **Lines:** 37

**Declarations:**

`fn main()`

---

## src/app_update.rs

**Language:** Rust | **Size:** 14.6 KB | **Lines:** 374

**Imports:**
- `std::collections::HashMap`
- `std::sync::atomic::{AtomicBool, AtomicU64, Ordering}`
- `std::sync::{Arc, RwLock}`
- `std::time::Duration`
- `anyhow::{bail, Context, Result}`
- `crate::config::ConfigurationService`

**Declarations:**

`const URL: &str = "https://raw.githubusercontent.com/zionyx/jenkins-tray/master/scripts/version.properties"`

`const UPDATE_DELAY: Duration = Duration::from_secs(5)`

`const UPDATE_PERIOD: Duration = Duration::from_secs(60 * 60)`

`const PROPERTY_VERSION_NUMBER: &str = "version.number"`

`const PROPERTY_INSTALLER_URL: &str = "version.installerUrl"`

**`impl std::fmt::Display for UpdateSource`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


`struct Version(u32, u32, u32, u32)`

**`impl Version`**
  `fn parse(text: &str) -> Result<Self>`


**`impl std::fmt::Display for Version`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


**`impl ApplicationUpdateService`**
  `pub fn new(runtime: tokio::runtime::Handle, config: Arc<ConfigurationService>) -> Arc<Self>`

  `pub fn on_new_version(&self, handler: Arc<dyn Fn(&str, &str) + Send + Sync>)`

  `pub fn initialize(self: Arc<Self>)`

  `pub fn enable_timer(self: Arc<Self>, enable: bool)`

  `fn stop_timer(&self)`

  `pub async fn check(&self, source: UpdateSource) -> Result<bool>`

  `async fn check_internal(&self) -> Result<bool>`


`fn current_version() -> Version`

`fn read_properties(text: &str) -> HashMap<String, String>`

`mod tests`

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

## src/errors.rs

**Language:** Rust | **Size:** 1.1 KB | **Lines:** 31

**Declarations:**

`mod tests`

---

## src/jenkins_client.rs

**Language:** Rust | **Size:** 49.5 KB | **Lines:** 1283

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

`fn split_origin(url: &str) -> Option<(&str, &str)>`

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

`fn json_scalar_to_string(value: &serde_json::Value) -> String`

`mod tests`

---

## src/main.rs

**Language:** Rust | **Size:** 99.3 KB | **Lines:** 2427

**Imports:**
- `std::cell::RefCell`
- `std::collections::{HashMap, HashSet}`
- `std::rc::Rc`
- `std::sync::{Arc, Mutex, MutexGuard}`
- `anyhow::Result`
- `chrono::{Duration, Local, TimeZone}`
- `slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel}`
- `app_update::{ApplicationUpdateService, UpdateSource}`
- `config::ConfigurationService`
- `model::{BuildCauseEnum, BuildDetails, BuildStatusEnum, Configuration, Project, Server}`
- *... and 3 more imports*

**Declarations:**

`mod app_update`

`mod config`

`mod errors`

`mod jenkins_client`

`mod model`

`mod notifications`

`mod projects_updater`

`mod tray`

`struct EditServerContext`
> Fields: `index: Option<usize>`

`enum SortKey`
> Variants: `Name`, `Details`, `LastBuild`, `LastSuccess`, `LastFailure`

`struct GridSort`
> Fields: `key: SortKey`, `ascending: bool`

`fn next_sort(current: Option<GridSort>, key: SortKey) -> Option<GridSort>`

`struct AppState`
> Fields: `row_keys: Vec<Option<(String, String)>>`, `sort: Option<GridSort>`, `settings_projects: HashMap<String, Vec<Project>>`, `edit_server: EditServerContext`, `auth_token_target: Option<(String, String)>`, `claim_target: Option<(String, String, u32)>`, `build_params_target: Option<(String, String)>`, `build_params: Vec<(String, String)>`

`struct Services`
> Fields: `config: Arc<ConfigurationService>`, `notifications: Arc<NotificationService>`, `clients: jenkins_client::Clients`, `updater: Rc<RefCell<ProjectsUpdateService>>`, `state: Arc<Mutex<AppState>>`, `tray: Arc<Mutex<TrayHub>>`, `app_update: Arc<ApplicationUpdateService>`, `runtime: tokio::runtime::Handle`

`fn lock_state(state: &Arc<Mutex<AppState>>) -> MutexGuard<'_, AppState>`

`struct Windows`
> Fields: `settings: Rc<SettingsWindow>`, `edit_server: Rc<EditServerWindow>`, `about: Rc<AboutWindow>`, `auth_token: Rc<AuthTokenWindow>`, `claim_build: Rc<ClaimBuildWindow>`, `build_params: Rc<BuildParamsWindow>`, `message_box: Rc<MessageBoxWindow>`

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

`fn display_name(project: &Project, show_display_name: bool) -> String`

`fn status_tooltip(project: &Project) -> String`

`fn status_resource(key: &str) -> Option<&'static str>`

`fn project_row(project: &Project, server: &Server, show_display_name: bool) -> ProjectRow`

`fn build_rows( config: &Configuration, sort: Option<GridSort>, ) -> (Vec<ProjectRow>, Vec<Option<(String, String)>>)`

`fn compare_projects( a: &Project, b: &Project, key: SortKey, show_display_name: bool, ) -> std::cmp::Ordering`

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

`static MESSAGE_BOX_YES_ACTION: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None)`

`fn take_message_box_yes_action() -> Option<Box<dyn Fn() + Send>>`

`fn show_message_box(message_box: &slint::Weak<MessageBoxWindow>, caption: &str, message: &str)`

`fn show_yes_no_message_box( message_box: &slint::Weak<MessageBoxWindow>, caption: &str, message: &str, on_yes: Box<dyn Fn() + Send>, )`

`fn wire_callbacks(ui: &AppWindow, services: &Services, windows: &Windows)`

`fn show_or_focus(ui: &AppWindow)`

`fn handle_tray_event( event: tray::TrayEvent, ui: &AppWindow, services: &Services, windows: &Windows, )`

`fn main() -> Result<()>`

`mod tests`

---

## src/model.rs

**Language:** Rust | **Size:** 17.5 KB | **Lines:** 459

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

  `pub fn resource_key(&self) -> String`

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

**Language:** Rust | **Size:** 8.4 KB | **Lines:** 242

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

## src/tray/ksni_backend.rs

**Language:** Rust | **Size:** 9.4 KB | **Lines:** 294

**Imports:**
- `std::collections::HashMap`
- `anyhow::{Context, Result}`
- `crate::model::BuildStatus`
- `super::{
    MenuItem, NotificationKind, TrayBackend, TrayEvent, TrayEventSender, TRAY_MENU_ITEMS,
    status_icon_rgba,
}`

**Declarations:**

`const ICON_SIZES: [u32`

**`impl KsniBackend`**
  `pub fn new() -> Self`


**`impl TrayBackend for KsniBackend`**
  `fn initialize(&mut self) -> Result<()>`

  `fn update_icon(&self, status: &BuildStatus) -> Result<()>`

  `fn set_tooltip(&self, text: &str) -> Result<()>`

  `fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()>`

  `fn set_context_menu(&self, items: Vec<MenuItem>) -> Result<()>`

  `fn set_event_sender(&mut self, sender: TrayEventSender)`

  `fn can_show_icon(&self) -> bool`

  `fn exit(&self) -> Result<()>`


`struct SniTray`
> Fields: `icons: Vec<ksni::Icon>`, `tooltip: String`, `menu_items: Vec<MenuItem>`, `sender: Option<TrayEventSender>`

**`impl SniTray`**
  `fn new() -> Self`


**`impl ksni::Tray for SniTray`**
  `fn id(&self) -> String`

  `fn category(&self) -> ksni::Category`

  `fn status(&self) -> ksni::Status`

  `fn title(&self) -> String`

  `fn icon_name(&self) -> String`

  `fn icon_pixmap(&self) -> Vec<ksni::Icon>`

  `fn tool_tip(&self) -> ksni::ToolTip`

  `fn activate(&mut self, _x: i32, _y: i32)`

  `fn menu(&self) -> Vec<ksni::MenuItem<Self>>`

  `fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool`

  `fn watcher_online(&self)`


`fn render_icons(status: &BuildStatus) -> Vec<ksni::Icon>`

`fn watcher_on_bus() -> bool`

`fn notify(title: &str, message: &str, kind: NotificationKind) -> Result<()>`

---

## src/tray/win32_backend.rs

**Language:** Rust | **Size:** 17.2 KB | **Lines:** 472

**Imports:**
- `std::sync::atomic::{AtomicBool, Ordering}`
- `std::sync::Mutex`
- `anyhow::{anyhow, Context, Result}`
- `windows::core::{w, PCWSTR}`
- `windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, TRUE, WPARAM}`
- `windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
}`
- `windows::Win32::System::LibraryLoader::GetModuleHandleW`
- `windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
}`
- `windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, GetCursorPos, GetWindowLongPtrW, ICONINFO,
    LoadCursorW, PostMessageW, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW,
    TrackPopupMenuEx, WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP,
    WNDCLASSEXW, GWLP_USERDATA, HICON, HMENU, HWND_MESSAGE, IDC_ARROW, MF_CHECKED, MF_GRAYED,
    MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TPM_RETURNCMD,
    WINDOW_EX_STYLE, WINDOW_STYLE,
}`
- `super::{
    status_icon_rgba, MenuItem, NotificationKind, TrayBackend, TrayEvent, TrayEventSender,
    TRAY_MENU_ITEMS,
}`
- *... and 1 more imports*

**Declarations:**

`const WM_TRAYICON: u32 = WM_APP + 1`

`const TRAY_UID: u32 = 1`

`const BALLOON_TIMEOUT_MS: u32 = 3000`

`const TOOLTIP_UNITS: usize = 127`

`const CLASS_NAME: PCWSTR = w!("JenkinsTrayTrayIcon")`

`const WINDOW_NAME: PCWSTR = w!("Jenkins Tray")`

`struct TrayCtx`
> Fields: `sender: Mutex<Option<TrayEventSender>>`, `menu: Mutex<Option<HMENU>>`

**`unsafe impl Send for Win32Backend`**

**`unsafe impl Sync for Win32Backend`**

**`impl Win32Backend`**
  `pub fn new() -> Self`

  `fn new_data(&self) -> Result<NOTIFYICONDATAW>`

  `fn require_icon(&self) -> Result<NOTIFYICONDATAW>`


**`impl TrayBackend for Win32Backend`**
  `fn initialize(&mut self) -> Result<()>`

  `fn update_icon(&self, status: &BuildStatus) -> Result<()>`

  `fn set_tooltip(&self, text: &str) -> Result<()>`

  `fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()>`

  `fn set_context_menu(&self, items: Vec<MenuItem>) -> Result<()>`

  `fn set_event_sender(&mut self, sender: TrayEventSender)`

  `fn can_show_icon(&self) -> bool`

  `fn exit(&self) -> Result<()>`


**`impl Drop for Win32Backend`**
  `fn drop(&mut self)`


`unsafe extern "system" fn wnd_proc( hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, ) -> LRESULT`

`unsafe fn ctx_of(hwnd: HWND) -> Option<&'static TrayCtx>`

`unsafe fn send_event(hwnd: HWND, event: TrayEvent)`

`unsafe fn show_menu(hwnd: HWND)`

`fn wide(text: &str) -> Vec<u16>`

`fn copy_units(dest: &mut [u16], text: &str, max_units: usize)`

`unsafe fn create_icon(rgba: &[u8], width: i32, height: i32) -> Result<HICON>`

---

## src/tray.rs

**Language:** Rust | **Size:** 22.9 KB | **Lines:** 619

**Imports:**
- `std::collections::HashMap`
- `anyhow::Result`
- `crate::model::{AllBuildDetails, BuildStatus, BuildStatusEnum, Configuration, Project}`
- `crate::notifications::TrayNotifier`

**Declarations:**

`mod ksni_backend`

`mod win32_backend`

**`impl TrayBackend for StubTrayBackend`**
  `fn initialize(&mut self) -> Result<()>`

  `fn update_icon(&self, status: &BuildStatus) -> Result<()>`

  `fn set_tooltip(&self, text: &str) -> Result<()>`

  `fn show_notification(&self, title: &str, message: &str, kind: NotificationKind) -> Result<()>`

  `fn set_context_menu(&self, _items: Vec<MenuItem>) -> Result<()>`

  `fn set_event_sender(&mut self, sender: TrayEventSender)`

  `fn can_show_icon(&self) -> bool`

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

  `pub fn set_event_sender(&mut self, sender: TrayEventSender)`

  `pub fn can_show_icon(&self) -> bool`


`mod tests`

