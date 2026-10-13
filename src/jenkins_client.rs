//! Jenkins API client.
//!
//! Port of C# `JenkinsTray.JenkinsService`: downloading the job list,
//! refreshing a single project (job state + the four build-detail documents),
//! triggering/stop builds and cancelling queue items, plus the URL-XML cache
//! with its per-cycle recycle.
//!
//! Differences from the original, noted explicitly:
//! - A 30 second request timeout is configured (the original `WebClient`
//!   could block forever).
//! - Build/stop requests append `token`/`cause` through a real query string;
//!   the original concatenates `&token=` onto `lastBuild/stop` (no `?`), which
//!   produces an invalid URL.
//! - `duration` is read from the `duration` XML element; the original parses
//!   `duration` but then assigns `estimatedDuration` twice (copy/paste bug).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use roxmltree::{Document, Node};

use crate::model::{
    AllBuildDetails, BuildCauses, BuildCause, BuildCauseEnum, BuildDetails, BuildStatus,
    ClaimDetails, Project, ProjectState, Server,
};

/// The original `JenkinsService.buildDetailsFilter` tree filter.
pub const BUILD_DETAILS_FILTER: &str = "?tree=number,fullDisplayName,timestamp,estimatedDuration,duration,result,userNodes,culprits[fullName],builtOn,url,actions[assignedBy,claimDate,claimed,claimedBy,reason]";

/// The tree filter used for every job list (top level and folders).
const JOB_LIST_TREE: &str = "api/xml?tree=jobs[name,url,color]";

// ---------------------------------------------------------------------------
// HTTP clients
// ---------------------------------------------------------------------------

/// The two pooled HTTP clients: one validating TLS certificates strictly and
/// one accepting untrusted certificates (for servers that have
/// `ignoreUntrustedCertificate` enabled - the original installs a per-server
/// certificate validation callback).
#[derive(Clone)]
pub struct Clients {
    strict: reqwest::Client,
    insecure: reqwest::Client,
}

impl Clients {
    pub fn new() -> Result<Self> {
        Ok(Self {
            strict: build_client(false)?,
            insecure: build_client(true)?,
        })
    }

    /// The client to use for a server.
    pub fn for_server(&self, server: &Server) -> &reqwest::Client {
        if server.ignore_untrusted_certificate {
            &self.insecure
        } else {
            &self.strict
        }
    }
}

/// Builds a pooled reqwest client (original: the per-thread
/// `CookieAwareWebClient` with the certificate validation callback).
pub fn build_client(ignore_untrusted_certificate: bool) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .danger_accept_invalid_certs(ignore_untrusted_certificate)
        .cookie_store(true)
        // The original has no timeout; bound it so a dead server cannot stall
        // a whole update cycle forever.
        .timeout(Duration::from_secs(30))
        .build()?)
}

/// The credentials of a server (original: `Server.Credentials`, present only
/// when a username is configured).
#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

impl From<&Server> for Credentials {
    fn from(server: &Server) -> Self {
        if server.username.as_deref().is_some_and(|u| !u.is_empty()) {
            Credentials {
                username: server.username.clone(),
                password: server.password.clone(),
            }
        } else {
            Credentials::default()
        }
    }
}

fn authenticated(req: reqwest::RequestBuilder, credentials: &Credentials) -> reqwest::RequestBuilder {
    match credentials.username.as_deref() {
        Some(username) => req.basic_auth(username, credentials.password.as_deref()),
        None => req,
    }
}

async fn get_text(
    client: &reqwest::Client,
    credentials: &Credentials,
    url: &str,
) -> Result<String> {
    let request = authenticated(client.get(url), credentials);
    let response = request.send().await?.error_for_status()?;
    Ok(response.text().await?)
}

/// The original `UploadString`: POST an empty body with the
/// `application/x-www-form-urlencoded` content type.
async fn upload_string(
    credentials: &Credentials,
    request: reqwest::RequestBuilder,
) -> Result<String> {
    let request = authenticated(request, credentials).header(
        reqwest::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded",
    );
    let response = request.send().await?.error_for_status()?;
    Ok(response.text().await?)
}

// ---------------------------------------------------------------------------
// URL helpers (original: NetUtils)
// ---------------------------------------------------------------------------

/// Joins URL fragments with a single `/` (original: NetUtils.ConcatUrls).
pub fn concat_urls(base: &str, fragments: &[&str]) -> String {
    let mut res = base.trim_end_matches('/').to_string();
    for fragment in fragments {
        res.push('/');
        res.push_str(fragment.trim_start_matches('/'));
    }
    res
}

/// Splits an absolute URL into its `scheme://host[:port]` origin and the
/// path (including query), e.g. `("http://h:8080", "/job/x/?a=1")`.
fn split_origin(url: &str) -> Option<(&str, &str)> {
    let scheme_end = url.find("://")?;
    let after_scheme = scheme_end + 3;
    match url[after_scheme..].find('/') {
        Some(offset) => {
            let split = after_scheme + offset;
            Some((&url[..split], &url[split..]))
        }
        None => Some((url, "")),
    }
}

/// Re-roots an absolute URL at the origin of the configured server.
///
/// Jenkins builds every URL in its answers (job lists, build links, ...) from
/// its own "Jenkins URL" system property. Here that value contains the short
/// hostname `eudeued01rud001`, which only resolves with the corporate DNS
/// suffix search the Windows resolver applies - the WSL/Linux resolver has no
/// search domain, so every URL taken verbatim from a Jenkins answer fails
/// with a DNS error while the FQDN the user entered in the configuration
/// works. All requests therefore use the origin from the configuration and
/// only keep the path from the Jenkins answer.
///
/// The original application has no such step: it runs on Windows, where
/// `getaddrinfo` appends the primary DNS suffix before giving up
/// (deviation from the original).
pub fn rebase_url(url: &str, server: &Server) -> String {
    let server_origin = split_origin(&server.url).map(|(origin, _)| origin).unwrap_or("");
    match split_origin(url) {
        // Absolute URL on a different origin: keep only its path.
        Some((origin, path)) if !server_origin.is_empty() && origin != server_origin => {
            format!("{server_origin}{path}")
        }
        // Relative answer: attach it to the configured server.
        None => concat_urls(&server.url, &[url]),
        // Already on the configured origin.
        Some(_) => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// XML cache (original: JenkinsService.cache / visitedURLs / RecycleCache)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct XmlCache {
    /// key = url, value = downloaded xml
    entries: HashMap<String, String>,
    /// URLs visited since the last recycle
    visited: HashSet<String>,
}

/// Shared XML cache: build-details documents are immutable per build URL, so
/// they are reused across update cycles until their URL stops being visited.
pub type SharedCache = Arc<Mutex<XmlCache>>;

pub fn new_cache() -> SharedCache {
    Arc::new(Mutex::new(XmlCache::default()))
}

/// Downloads `url`, consulting and filling the cache for cacheable documents
/// (original: DownloadString with `cacheable = true`).
async fn download(
    client: &reqwest::Client,
    credentials: &Credentials,
    url: &str,
    cache: Option<&SharedCache>,
) -> Result<String> {
    if let Some(cache) = cache {
        let mut cache = cache.lock().expect("xml cache poisoned");
        cache.visited.insert(url.to_string());
        if let Some(cached) = cache.entries.get(url) {
            log::trace!("Cache hit: {url}");
            return Ok(cached.clone());
        }
        log::trace!("Cache miss: {url}");
    }

    log::trace!("Downloading: {url}");
    let text = get_text(client, credentials, url).await?;

    if let Some(cache) = cache {
        let mut cache = cache.lock().expect("xml cache poisoned");
        cache.entries.insert(url.to_string(), text.clone());
    }
    Ok(text)
}

/// Keeps only the entries visited during the last cycle
/// (original: JenkinsService.RecycleCache).
pub fn recycle_cache(cache: &SharedCache) {
    let mut cache = cache.lock().expect("xml cache poisoned");
    let visited = std::mem::take(&mut cache.visited);
    cache.entries.retain(|url, _| visited.contains(url));
}

/// Drops one document from the cache
/// (original: JenkinsService.RemoveFromCache).
pub fn remove_cache_entry(cache: &SharedCache, url: &str) {
    let mut cache = cache.lock().expect("xml cache poisoned");
    cache.entries.remove(url);
    cache.visited.remove(url);
}

// ---------------------------------------------------------------------------
// XML parsing helpers
// ---------------------------------------------------------------------------

fn element_children<'doc, 'input>(node: Node<'doc, 'input>) -> impl Iterator<Item = Node<'doc, 'input>> {
    node.children().filter(|child| child.is_element())
}

fn find_child<'doc, 'input>(node: Node<'doc, 'input>, name: &str) -> Option<Node<'doc, 'input>> {
    element_children(node).find(|child| child.tag_name().name() == name)
}

fn child_text<'doc, 'input>(node: Node<'doc, 'input>, name: &str) -> Option<&'doc str> {
    find_child(node, name).and_then(|child| child.text())
}

fn child_bool<'doc, 'input>(node: Node<'doc, 'input>, name: &str) -> Option<bool> {
    child_text(node, name).map(|text| text == "true")
}

/// `StringUtils.ExtractUserName`: cuts " <mail>" and " (Jenkins)" suffixes.
fn extract_user_name(full_name: &str) -> String {
    let mut res = full_name;

    if let Some(index) = res.find(" <") && index > 0 {
        res = &res[..index];
    }
    if let Some(index) = res.find(" (") && index > 0 {
        res = &res[..index];
    }
    res.trim().to_string()
}

// ---------------------------------------------------------------------------
// Job list (original: JenkinsService.LoadProjects / GetProjects)
// ---------------------------------------------------------------------------

/// One entry of a `jobs[name,url,color]` list. `color` is `None` for folders.
#[derive(Debug, Clone)]
pub struct JobEntry {
    pub name: String,
    pub url: String,
    pub color: Option<String>,
}

/// Parses a job list document (root `<hudson>`, `<folder>` or
/// `<workflowMultiBranchProject>`, all addressed through their `job` children).
pub fn parse_job_list(xml: &str) -> Result<Vec<JobEntry>> {
    let document = Document::parse(xml).context("invalid job list XML")?;
    let root = document.root_element();
    let jobs = element_children(root)
        .filter(|child| child.tag_name().name() == "job")
        .filter_map(|job| {
            Some(JobEntry {
                name: child_text(job, "name")?.to_string(),
                url: child_text(job, "url")?.to_string(),
                color: child_text(job, "color").map(String::from),
            })
        })
        .collect();
    Ok(jobs)
}

/// Loads all jobs of a server, recursing into folders
/// (original: JenkinsService.LoadProjects).
pub async fn load_projects(client: &reqwest::Client, server: &Server) -> Result<Vec<Project>> {
    let credentials = Credentials::from(server);
    let url = concat_urls(&server.url, &[JOB_LIST_TREE]);
    log::info!("Loading projects from {url}");

    let mut projects = Vec::new();
    collect_projects(client, &credentials, &url, server, &mut projects).await?;
    log::info!("Done loading projects");
    Ok(projects)
}

/// Recursion over folders: boxed because `async fn` futures cannot refer to
/// themselves directly.
fn collect_projects<'a>(
    client: &'a reqwest::Client,
    credentials: &'a Credentials,
    list_url: &'a str,
    server: &'a Server,
    projects: &'a mut Vec<Project>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let xml = get_text(client, credentials, list_url).await?;
        for job in parse_job_list(&xml)? {
            let job_url = rebase_url(&job.url, server);
            match job.color {
                // A real job: keep it (the original de-duplicates by server+name).
                Some(_) => {
                    if !projects.iter().any(|project| project.name == job.name) {
                        log::debug!("Found project {} ({})", job.name, job_url);
                        projects.push(Project {
                            name: job.name,
                            url: job_url,
                            server: Some(server.url.clone()),
                            ..Default::default()
                        });
                    }
                }
                // No color = folder: recurse into its own job list.
                None => {
                    let url = concat_urls(&job_url, &[JOB_LIST_TREE]);
                    collect_projects(client, credentials, &url, server, projects).await?;
                }
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Project refresh (original: JenkinsService.UpdateProject)
// ---------------------------------------------------------------------------

/// The fields of a job's `/api/xml` document used by `UpdateProject`.
#[derive(Debug, Default)]
pub struct JobState {
    pub display_name: Option<String>,
    pub in_queue: bool,
    pub queue_id: Option<u64>,
    pub in_queue_since: Option<i64>,
    pub queue_why: Option<String>,
    pub queue_stuck: bool,
    pub color: String,
    pub last_build_url: Option<String>,
    pub last_completed_build_url: Option<String>,
    pub last_successful_build_url: Option<String>,
    pub last_failed_build_url: Option<String>,
}

/// Parses the job document produced by `GET {job}/api/xml`.
pub fn parse_job_state(xml: &str) -> Result<JobState> {
    let document = Document::parse(xml).context("invalid job XML")?;
    let root = document.root_element();

    let queue = find_child(root, "queueItem");
    let color = child_text(root, "color")
        .ok_or_else(|| anyhow!("job XML has no color"))?
        .to_string();

    Ok(JobState {
        display_name: child_text(root, "displayName").map(String::from),
        in_queue: child_bool(root, "inQueue").unwrap_or(false),
        queue_id: queue.and_then(|q| child_text(q, "id")).and_then(|id| id.parse().ok()),
        in_queue_since: queue
            .and_then(|q| child_text(q, "inQueueSince"))
            .and_then(|ms| ms.parse().ok()),
        queue_why: queue.and_then(|q| child_text(q, "why")).map(String::from),
        queue_stuck: queue.and_then(|q| child_bool(q, "stuck")).unwrap_or(false),
        color,
        last_build_url: find_child(root, "lastBuild").and_then(|b| child_text(b, "url")).map(String::from),
        last_completed_build_url: find_child(root, "lastCompletedBuild")
            .and_then(|b| child_text(b, "url"))
            .map(String::from),
        last_successful_build_url: find_child(root, "lastSuccessfulBuild")
            .and_then(|b| child_text(b, "url"))
            .map(String::from),
        last_failed_build_url: find_child(root, "lastFailedBuild")
            .and_then(|b| child_text(b, "url"))
            .map(String::from),
    })
}

/// Refreshes one project: downloads the job document and the four build
/// detail documents (original: JenkinsService.UpdateProject).
///
/// `Ok` carries the new state; `details` is `None` when the job document was
/// read but a build-details download failed - the caller then keeps the
/// previous details while still applying the queue/display-name updates, just
/// like the original (which mutates the shared project before the failing
/// call throws).
pub async fn update_project(
    client: &reqwest::Client,
    server: &Server,
    project: &Project,
    cache: &SharedCache,
) -> Result<ProjectState> {
    let credentials = Credentials::from(server);
    // The job document is requested at the configured origin even when the
    // project URL saved in the configuration uses another host name.
    let url = concat_urls(&rebase_url(&project.url, server), &["api/xml"]);
    let xml = download(client, &credentials, &url, None).await?;
    let job = parse_job_state(&xml)?;

    let status = BuildStatus::from_jenkins_color(&job.color, job.queue_stuck);
    let mut details = AllBuildDetails {
        status,
        previous_status: None,
        last_build: None,
        last_completed_build: None,
        last_successful_build: None,
        last_failed_build: None,
    };

    let mut all_builds_loaded = true;
    let slots: [(&mut Option<BuildDetails>, Option<String>); 4] = [
        (&mut details.last_build, job.last_build_url),
        (
            &mut details.last_completed_build,
            job.last_completed_build_url,
        ),
        (
            &mut details.last_successful_build,
            job.last_successful_build_url,
        ),
        (&mut details.last_failed_build, job.last_failed_build_url),
    ];
    for (slot, build_url) in slots {
        let Some(build_url) = build_url else {
            continue;
        };
        // Build links in the job document also use the Jenkins-internal host.
        let build_url = rebase_url(&build_url, server);
        match get_build_details(client, &credentials, &build_url, cache).await {
            Ok(build) => *slot = Some(build),
            Err(error) => {
                log::error!(
                    "Error getting build details from {build_url}: {}",
                    crate::errors::root_cause(&error)
                );
                all_builds_loaded = false;
            }
        }
    }

    Ok(ProjectState {
        display_name: job.display_name,
        in_queue: job.in_queue,
        queue_id: job.queue_id,
        queue_why: job.queue_why,
        // The original has no stored queue-stuck flag (it feeds the status
        // only); the Rust model keeps it, so it is refreshed on every poll.
        queue_stuck: Some(job.queue_stuck),
        in_queue_since: job.in_queue_since,
        details: if all_builds_loaded {
            Some(details)
        } else {
            None
        },
    })
}

// ---------------------------------------------------------------------------
// Build details (original: JenkinsService.GetBuildDetails)
// ---------------------------------------------------------------------------

/// Parses one build document (`GET {build}/api/xml?tree=...`).
pub fn parse_build_details(xml: &str) -> Result<BuildDetails> {
    let document = Document::parse(xml).context("invalid build XML")?;
    let root = document.root_element();

    let number = child_text(root, "number")
        .ok_or_else(|| anyhow!("build XML has no number"))?
        .parse::<u32>()
        .context("invalid build number")?;
    let timestamp = child_text(root, "timestamp")
        .ok_or_else(|| anyhow!("build XML has no timestamp"))?
        .parse::<i64>()
        .context("invalid build timestamp")?;

    let mut users = HashSet::new();
    for child in element_children(root) {
        match child.tag_name().name() {
            "culprit" => {
                if let Some(full_name) = child_text(child, "fullName") {
                    users.insert(extract_user_name(full_name));
                }
            }
            // Some Jenkins versions wrap the culprits in a container element.
            "culprits" => {
                for wrapped in element_children(child) {
                    match wrapped.tag_name().name() {
                        "fullName" => {
                            if let Some(full_name) = wrapped.text() {
                                users.insert(extract_user_name(full_name));
                            }
                        }
                        "culprit" => {
                            if let Some(full_name) = child_text(wrapped, "fullName") {
                                users.insert(extract_user_name(full_name));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let result = child_text(root, "result").unwrap_or("");

    Ok(BuildDetails {
        number,
        display_name: child_text(root, "fullDisplayName").unwrap_or("").to_string(),
        time: timestamp,
        estimated_duration: child_text(root, "estimatedDuration").and_then(|v| v.parse().ok()),
        duration: child_text(root, "duration").and_then(|v| v.parse().ok()),
        result: BuildStatus::from_jenkins_result(result),
        users,
        causes: Some(fill_in_build_causes(root)),
        claim_details: fill_in_claim_details(root),
    })
}

/// Downloads and parses one build document, using the cache
/// (original: GetBuildDetails with `cacheable = true`).
async fn get_build_details(
    client: &reqwest::Client,
    credentials: &Credentials,
    build_url: &str,
    cache: &SharedCache,
) -> Result<BuildDetails> {
    let url = concat_urls(build_url, &["api/xml", BUILD_DETAILS_FILTER]);
    log::debug!("Getting build details from {url}");
    let xml = download(client, credentials, &url, Some(cache)).await?;
    parse_build_details(&xml)
}

/// Port of `BuildCauses.FillInBuildCauses`.
fn fill_in_build_causes(root: Node<'_, '_>) -> BuildCauses {
    let mut causes: Vec<BuildCause> = Vec::new();

    for action in element_children(root).filter(|n| n.tag_name().name() == "action") {
        for cause_node in element_children(action).filter(|n| n.tag_name().name() == "cause") {
            let short_description = child_text(cause_node, "shortDescription").unwrap_or("");
            let mut cause = BuildCause {
                cause: BuildCauseEnum::Unknown,
                short_description: short_description.to_string(),
                starter: None,
                user_id: None,
            };

            let short_lower = short_description.to_lowercase();
            if short_description.starts_with("Started by user") {
                cause.cause = BuildCauseEnum::User;
                cause.starter = non_empty(child_text(cause_node, "userName")).map(String::from);
                cause.user_id = non_empty(child_text(cause_node, "userId")).map(String::from);
            } else if short_lower.starts_with("started by timer") {
                cause.cause = BuildCauseEnum::Timer;
            } else if short_lower.starts_with("started by upstream project") {
                cause.cause = BuildCauseEnum::UpstreamProject;
                cause.starter =
                    non_empty(child_text(cause_node, "upstreamProject")).map(String::from);
            } else if short_lower.starts_with("started by an scm change") {
                cause.cause = BuildCauseEnum::SCM;
            } else if short_lower.starts_with("started by remote host") {
                cause.cause = BuildCauseEnum::RemoteHost;
                // "Started by remote host 1.2.3.4 with note: xxx"
                let mut remote_host = short_description;
                let prefix = "Started by remote host ";
                if remote_host.len() >= prefix.len() {
                    remote_host = &remote_host[prefix.len()..];
                }
                if let Some(index) = remote_host.find(" with note: ") && index > 0 {
                    remote_host = &remote_host[..index];
                }
                cause.starter = Some(remote_host.to_string());
            }

            causes.push(cause);
        }
    }

    // original: HasUniqueCauses - the first cause defines the expected kind,
    // any different later cause clears the flag.
    let mut unique: Option<bool> = None;
    let mut expected = BuildCauseEnum::Unknown;
    for cause in &causes {
        match unique {
            None => {
                expected = cause.cause;
                unique = Some(true);
            }
            Some(_) if cause.cause != expected => unique = Some(false),
            _ => {}
        }
    }

    BuildCauses {
        causes,
        has_unique_causes: unique,
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// Port of `ClaimService.FillInBuildDetails`.
fn fill_in_claim_details(root: Node<'_, '_>) -> Option<ClaimDetails> {
    let claimed_action = element_children(root).find(|node| {
        node.tag_name().name() == "action" && child_bool(*node, "claimed") == Some(true)
    })?;
    Some(ClaimDetails {
        assignor: child_text(claimed_action, "assignedBy").unwrap_or("").to_string(),
        assignee: child_text(claimed_action, "claimedBy").unwrap_or("").to_string(),
        reason: child_text(claimed_action, "reason").unwrap_or("").to_string(),
    })
}

// ---------------------------------------------------------------------------
// Actions (original: SafeRunBuild / SafeStopBuild / SafeRemoveFromQueue)
// ---------------------------------------------------------------------------

/// Triggers a build (original: JenkinsService.RunBuild).
pub async fn trigger_build(
    client: &reqwest::Client,
    server: &Server,
    project: &Project,
) -> Result<()> {
    let credentials = Credentials::from(server);
    let url = concat_urls(&rebase_url(&project.url, server), &["build?delay=0sec"]);
    log::info!("Running build at {url}");

    let mut request = client.post(&url);
    if let Some(token) = non_empty(project.authentication_token.as_deref()) {
        request = request.query(&[("token", token)]);
        if let Some(cause) = non_empty(project.cause_text.as_deref()) {
            request = request.query(&[("cause", cause)]);
        }
    }
    upload_string(&credentials, request).await?;
    log::info!("Done running build");
    Ok(())
}

/// Stops the last build (original: JenkinsService.StopBuild).
pub async fn stop_build(
    client: &reqwest::Client,
    server: &Server,
    project: &Project,
) -> Result<()> {
    let credentials = Credentials::from(server);
    let url = concat_urls(&rebase_url(&project.url, server), &["lastBuild/stop"]);
    log::info!("Stopping build at {url}");

    let mut request = client.post(&url);
    if let Some(token) = non_empty(project.authentication_token.as_deref()) {
        request = request.query(&[("token", token)]);
        if let Some(cause) = non_empty(project.cause_text.as_deref()) {
            request = request.query(&[("cause", cause)]);
        }
    }
    upload_string(&credentials, request).await?;
    log::info!("Done stopping build");
    Ok(())
}

/// Removes a queued item (original: JenkinsService.RemoveFromQueue).
///
/// A 404 response is swallowed as a workaround for JENKINS-21311, exactly
/// like the original.
pub async fn remove_from_queue(
    client: &reqwest::Client,
    server: &Server,
    queue_id: u64,
) -> Result<()> {
    let credentials = Credentials::from(server);
    let url = concat_urls(&server.url, &["queue/cancelItem"]);
    log::info!("Removing queue item at {url}?id={queue_id}");

    let request = client.post(&url).query(&[("id", queue_id)]);
    let response = authenticated(request, &credentials)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // consume error 404 (JENKINS-21311)
        return Ok(());
    }
    response.error_for_status()?;
    log::info!("Done removing queue item");
    Ok(())
}

// ---------------------------------------------------------------------------
// Build parameters (new feature - not in the original C# application)
// ---------------------------------------------------------------------------

/// The widget kind of a job parameter, mapped from the Jenkins `type` field
/// of a parameter definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// `StringParameterDefinition` / `TextParameterDefinition` - and the
    /// fallback for every unknown type, including unochoice's
    /// `ChoiceParameter`, whose choices only exist in its runtime script.
    Text,
    /// `ChoiceParameterDefinition` with a static `choices` array.
    Choice,
    /// `BooleanParameterDefinition`.
    Boolean,
    /// `PasswordParameterDefinition`.
    Password,
}

/// One parameter definition of a job: the input model of the new
/// "Build with parameters" dialog.
#[derive(Debug, Clone)]
pub struct JobParameter {
    pub name: String,
    pub description: String,
    pub kind: ParamKind,
    /// The job's default value (empty when Jenkins reports none).
    pub default: String,
    /// The choices of a `ChoiceParameterDefinition` (empty otherwise).
    pub choices: Vec<String>,
}

/// Parses the parameter definitions out of a `<job>/api/json` answer.
///
/// Current Jenkins reports them under `property`, older versions under
/// `actions`; both are read (`property` first) and duplicates by name are
/// dropped, so a job is listed exactly once.
pub fn parse_build_parameters(json: &str) -> Result<Vec<JobParameter>> {
    let root: serde_json::Value =
        serde_json::from_str(json).context("Could not parse the job's JSON answer")?;

    let mut definitions: Vec<&serde_json::Value> = Vec::new();
    for key in ["property", "actions"] {
        if let Some(items) = root.get(key).and_then(|value| value.as_array()) {
            for item in items {
                if let Some(list) = item.get("parameterDefinitions").and_then(|v| v.as_array()) {
                    definitions.extend(list.iter());
                }
            }
        }
    }

    let mut parameters: Vec<JobParameter> = Vec::new();
    for definition in definitions {
        let Some(name) = definition.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        if parameters.iter().any(|p| p.name == name) {
            continue;
        }
        let type_name = definition
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let description = definition
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let default = definition
            .get("defaultParameterValue")
            .and_then(|value| value.get("value"))
            .map(json_scalar_to_string)
            .unwrap_or_default();
        let choices: Vec<String> = definition
            .get("choices")
            .and_then(|value| value.as_array())
            .map(|values| values.iter().map(json_scalar_to_string).collect())
            .unwrap_or_default();

        let kind = if type_name.contains("Boolean") {
            ParamKind::Boolean
        } else if type_name.contains("Password") {
            ParamKind::Password
        } else if type_name.contains("Choice") && !choices.is_empty() {
            ParamKind::Choice
        } else {
            // String/Text definitions, unochoice's `ChoiceParameter` and
            // anything unknown become a plain text input - the value can
            // then be typed like in the reference shell script.
            ParamKind::Text
        };

        // A choice must submit one of its choices: fall back to the first
        // choice when Jenkins reports no (or a stale) default.
        let (default, choices) =
            if kind == ParamKind::Choice && !choices.contains(&default) {
                (choices.first().cloned().unwrap_or_default(), choices)
            } else {
                (default, choices)
            };

        parameters.push(JobParameter {
            name: name.to_string(),
            description: description.to_string(),
            kind,
            default,
            choices,
        });
    }
    Ok(parameters)
}

/// Stringifies a JSON scalar used as a parameter value (`value` is a string
/// for most definitions, a JSON boolean for boolean definitions, and may be
/// null, e.g. for password parameters).
fn json_scalar_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Downloads the parameter definitions of a job for the new
/// "Build with parameters" dialog (`<job>/api/json` with a tree filter
/// selecting only the parameter definitions).
pub async fn get_build_parameters(
    client: &reqwest::Client,
    server: &Server,
    project: &Project,
) -> Result<Vec<JobParameter>> {
    let credentials = Credentials::from(server);
    let url = concat_urls(&rebase_url(&project.url, server), &["api/json"]);
    const TREE: &str = "property[parameterDefinitions[name,type,description,defaultParameterValue[value],choices]],actions[parameterDefinitions[name,type,description,defaultParameterValue[value],choices]]";
    log::info!("Loading build parameters of {} at {url}", project.name);

    let request = authenticated(client.get(&url), &credentials).query(&[("tree", TREE)]);
    let response = request.send().await?.error_for_status()?;
    let text = response.text().await?;
    parse_build_parameters(&text)
        .with_context(|| format!("Could not load the parameters of {}", project.name))
}

/// Triggers a parameterized build (new feature: the original `RunBuild`
/// posts `/build?delay=0sec`, which on a parameterized job only serves the
/// "Build with Parameters" form instead of queueing a run - the reference
/// script `jenkins_files/trigger_pipeline.sh` therefore uses
/// `buildWithParameters` with the values as form data, as this does).
///
/// Like `trigger_build`, an authentication token / cause text configured on
/// the project is appended as a query string.
pub async fn trigger_build_parameters(
    client: &reqwest::Client,
    server: &Server,
    project: &Project,
    params: &[(String, String)],
) -> Result<()> {
    let credentials = Credentials::from(server);
    let url = concat_urls(&rebase_url(&project.url, server), &["buildWithParameters"]);
    log::info!("Running parameterized build at {url}");

    let mut request = client.post(&url).form(params);
    if let Some(token) = non_empty(project.authentication_token.as_deref()) {
        request = request.query(&[("token", token)]);
        if let Some(cause) = non_empty(project.cause_text.as_deref()) {
            request = request.query(&[("cause", cause)]);
        }
    }
    // Jenkins answers 201 Created with the queue item location; `.form()`
    // sends the values as `application/x-www-form-urlencoded` body like
    // `curl --data` in the reference script.
    let response = authenticated(request, &credentials)
        .send()
        .await?
        .error_for_status()?;
    let _ = response.text().await?;
    log::info!("Done running build");
    Ok(())
}

/// Claims a build (original: `ClaimService.ClaimBuild`).
///
/// Posts the claim JSON (`assignee`/`reason`/`sticky`, `DataContractJsonSerializer`
/// format) as form data to `<buildUrl>claim/claim`. Redirects are not
/// followed (`AllowAutoRedirect = false`) and only 200/302 count as success;
/// the assignee comes from the session, so the configured credentials are
/// attached (the original passes `HttpWebRequest.Credentials` for the same
/// reason) and is empty without them ("currently supporting assigning to
/// self"). Afterwards the cached build document is dropped so the next
/// update re-reads the claim state (original: `RemoveFromCache(buildUrl)`).
///
/// The redirect policy is per-client in reqwest, so a dedicated client is
/// built for this call (the original request also has no shared cookie
/// container).
pub async fn claim_build(
    server: &Server,
    project: &Project,
    build_number: u32,
    reason: &str,
    sticky: bool,
    cache: &SharedCache,
) -> Result<()> {
    let credentials = Credentials::from(server);
    let project_url = rebase_url(&project.url, server);
    let number = build_number.to_string();
    let url = concat_urls(&project_url, &[&number, "claim", "claim"]);
    log::info!("Claiming build at {url}");

    let json = serde_json::json!({
        "assignee": credentials.username.clone().unwrap_or_default(),
        "reason": reason,
        "sticky": sticky,
    })
    .to_string();

    let no_redirect = reqwest::Client::builder()
        .danger_accept_invalid_certs(server.ignore_untrusted_certificate)
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = authenticated(no_redirect.post(&url), &credentials)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[("json", json)])
        .send()
        .await?;
    let status = response.status();
    if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::FOUND {
        return Err(anyhow!("Received response code {status}"));
    }

    // original: RemoveFromCache(buildUrl) - drop the cached build document.
    let build_url = concat_urls(&project_url, &[&number]);
    let details_url = concat_urls(&build_url, &["api/xml", BUILD_DETAILS_FILTER]);
    remove_cache_entry(cache, &details_url);

    log::info!("Done claiming build");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BuildStatusEnum;

    #[test]
    fn parses_job_list_with_folder() {
        let xml = r#"<hudson>
            <job><name>job-a</name><url>http://ci/job/job-a/</url><color>blue</color></job>
            <job><name>folder-a</name><url>http://ci/job/folder-a/</url></job>
        </hudson>"#;
        let jobs = parse_job_list(xml).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "job-a");
        assert_eq!(jobs[0].color.as_deref(), Some("blue"));
        assert!(jobs[1].color.is_none());
    }

    #[test]
    fn parses_job_state() {
        let xml = r#"<project>
            <displayName>My Job</displayName>
            <inQueue>true</inQueue>
            <queueItem>
                <id>42</id>
                <inQueueSince>1700000000000</inQueueSince>
                <why>waiting for next available executor</why>
                <stuck>false</stuck>
            </queueItem>
            <color>red_anime</color>
            <lastBuild><url>http://ci/job/j/12/</url></lastBuild>
            <lastCompletedBuild><url>http://ci/job/j/11/</url></lastCompletedBuild>
            <lastSuccessfulBuild><url>http://ci/job/j/9/</url></lastSuccessfulBuild>
            <lastFailedBuild><url>http://ci/job/j/11/</url></lastFailedBuild>
        </project>"#;
        let state = parse_job_state(xml).unwrap();
        assert_eq!(state.display_name.as_deref(), Some("My Job"));
        assert!(state.in_queue);
        assert_eq!(state.queue_id, Some(42));
        assert_eq!(state.in_queue_since, Some(1_700_000_000_000));
        assert_eq!(state.queue_why.as_deref(), Some("waiting for next available executor"));
        assert!(!state.queue_stuck);
        assert_eq!(state.color, "red_anime");
        assert_eq!(state.last_build_url.as_deref(), Some("http://ci/job/j/12/"));
        assert_eq!(state.last_failed_build_url.as_deref(), Some("http://ci/job/j/11/"));
    }

    #[test]
    fn parses_build_details_with_causes_and_claim() {
        let xml = r#"<freeStyleBuild>
            <number>12</number>
            <fullDisplayName>My Job #12</fullDisplayName>
            <timestamp>1700000000000</timestamp>
            <estimatedDuration>60000</estimatedDuration>
            <duration>90000</duration>
            <result>FAILURE</result>
            <culprit><fullName>Jane Doe &lt;jane@x&gt;</fullName></culprit>
            <culprit><fullName>Bob (Bobo)</fullName></culprit>
            <action>
                <cause>
                    <shortDescription>Started by an SCM change</shortDescription>
                </cause>
            </action>
            <action>
                <claimed>true</claimed>
                <assignedBy>alice</assignedBy>
                <claimedBy>bob</claimedBy>
                <reason>looking into it</reason>
            </action>
        </freeStyleBuild>"#;
        let build = parse_build_details(xml).unwrap();
        assert_eq!(build.number, 12);
        assert_eq!(build.result, BuildStatusEnum::Failed);
        assert_eq!(build.duration, Some(90_000));
        assert!(build.users.contains("Jane Doe"));
        assert!(build.users.contains("Bob"));

        let causes = build.causes.unwrap();
        assert_eq!(causes.has_unique_causes, Some(true));
        assert_eq!(causes.causes[0].cause, BuildCauseEnum::SCM);

        let claim = build.claim_details.unwrap();
        assert_eq!(claim.assignor, "alice");
        assert_eq!(claim.assignee, "bob");
        assert_eq!(claim.reason, "looking into it");
    }

    #[test]
    fn build_status_from_color() {
        assert_eq!(
            BuildStatus::from_jenkins_color("blue", false).value,
            BuildStatusEnum::Successful
        );
        let anime = BuildStatus::from_jenkins_color("red_anime", true);
        assert_eq!(anime.value, BuildStatusEnum::Failed);
        assert!(anime.is_in_progress);
        assert!(anime.is_stuck);
        assert_eq!(
            BuildStatus::from_jenkins_color("notbuilt", false).value,
            BuildStatusEnum::Unknown
        );
    }

    /// URLs taken from Jenkins answers (which use the host from the Jenkins
    /// URL setting) must be re-rooted at the configured server origin, while
    /// URLs already on that origin stay untouched.
    #[test]
    fn rebases_jenkins_urls_onto_the_configured_origin() {
        let server = Server {
            url: "http://jenkins.corp.example:8080/".into(),
            ..Default::default()
        };

        // Different origin (short host name from the Jenkins answer).
        assert_eq!(
            rebase_url("http://jenkins:8080/job/folder/", &server),
            "http://jenkins.corp.example:8080/job/folder/"
        );
        // Different scheme as well.
        assert_eq!(
            rebase_url("http://jenkins:8080/job/j/42/console", &server),
            "http://jenkins.corp.example:8080/job/j/42/console"
        );
        // Same origin: untouched.
        assert_eq!(
            rebase_url("http://jenkins.corp.example:8080/job/j/42/", &server),
            "http://jenkins.corp.example:8080/job/j/42/"
        );
        // Origin without a path.
        assert_eq!(
            rebase_url("http://jenkins:8080", &server),
            "http://jenkins.corp.example:8080"
        );
        // Relative answer: attached to the configured server.
        assert_eq!(
            rebase_url("job/x/", &server),
            "http://jenkins.corp.example:8080/job/x/"
        );
    }

    /// Loads the real project list from the first server configured on this
    /// machine (run manually: `cargo test live_load_projects -- --ignored
    /// --nocapture`). Asserts that every returned URL was rebased onto the
    /// configured origin - the regression that made the settings dialog fail
    /// with DNS errors for every folder Jenkins named with its short host.
    #[test]
    #[ignore = "requires the live Jenkins server from the local configuration"]
    fn live_load_projects_from_configuration() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let config_service = crate::config::ConfigurationService::new().expect("configuration");
        let config = config_service.get_config().expect("configuration");
        let server = config.servers.first().expect("a configured server");
        let client = build_client(server.ignore_untrusted_certificate).expect("client");

        let projects = runtime
            .block_on(load_projects(&client, server))
            .expect("load_projects against the live server");

        assert!(!projects.is_empty(), "expected at least one project");
        let origin = split_origin(&server.url).expect("server url has an origin").0;
        for project in &projects {
            assert!(
                project.url.starts_with(&format!("{origin}/")),
                "project url not rebased: {}",
                project.url
            );
        }
        println!(
            "loaded {} projects (first: {})",
            projects.len(),
            projects[0].url
        );

        // Poll one project as well: the job document and the build links
        // inside it must use the configured origin too.
        let state = runtime
            .block_on(update_project(
                &client,
                server,
                &projects[0],
                &new_cache(),
            ))
            .expect("update_project against the live server");
        let details = state.details.as_ref();
        println!(
            "polled {}: status {:?}, last build {:?}",
            projects[0].name,
            details.map(|d| d.status.value),
            details.and_then(|d| d.last_build.as_ref().map(|b| b.number))
        );
    }

    #[test]
    fn parses_build_parameter_definitions() {
        // Sampled from the live server (GammaRay_ps_test).
        let json = r#"{
            "_class": "org.jenkinsci.plugins.workflow.job.WorkflowJob",
            "property": [
                {"_class": "jenkins.model.BuildDiscarderProperty"},
                {"_class": "hudson.model.ParametersDefinitionProperty",
                 "parameterDefinitions": [
                    {"_class": "hudson.model.StringParameterDefinition",
                     "defaultParameterValue": {"_class": "hudson.model.StringParameterValue",
                                               "value": "v3.4.0"},
                     "description": null,
                     "name": "SCM_BRANCH",
                     "type": "StringParameterDefinition"},
                    {"_class": "hudson.model.ChoiceParameterDefinition",
                     "defaultParameterValue": {"_class": "hudson.model.StringParameterValue",
                                               "value": "QT6_9_2"},
                     "description": "Qt version",
                     "name": "QT_VERSION",
                     "type": "ChoiceParameterDefinition",
                     "choices": ["QT5_15_2", "QT6_9_2"]}
                 ]}
            ]
        }"#;
        let params = parse_build_parameters(json).unwrap();
        assert_eq!(params.len(), 2);

        assert_eq!(params[0].name, "SCM_BRANCH");
        assert_eq!(params[0].kind, ParamKind::Text);
        assert_eq!(params[0].default, "v3.4.0");
        assert_eq!(params[0].description, "");

        assert_eq!(params[1].name, "QT_VERSION");
        assert_eq!(params[1].kind, ParamKind::Choice);
        assert_eq!(params[1].default, "QT6_9_2");
        assert_eq!(params[1].choices, ["QT5_15_2", "QT6_9_2"]);
    }

    #[test]
    fn build_parameter_kind_fallbacks() {
        // unochoice reports `ChoiceParameter` without a static `choices`
        // array, boolean defaults are JSON booleans, password defaults may
        // be null, and definitions can live under `actions` (old Jenkins).
        let json = r#"{
            "actions": [
                {"parameterDefinitions": [
                    {"description": "GammaRay Git tag to build",
                     "name": "GAMMARAY_VERSION",
                     "type": "org.biouno.unochoice.ChoiceParameter",
                     "defaultParameterValue": {"value": "v3.4.1"}},
                    {"name": "CLEAN",
                     "type": "BooleanParameterDefinition",
                     "defaultParameterValue": {"value": true}},
                    {"name": "SECRET",
                     "type": "PasswordParameterDefinition",
                     "defaultParameterValue": {"value": null}},
                    {"name": "WHATEVER",
                     "type": "some.PluginParameterDefinition"}
                ]}
            ]
        }"#;
        let params = parse_build_parameters(json).unwrap();
        assert_eq!(params.len(), 4);

        // The unochoice choices are unknown at rest -> typed as text.
        assert_eq!(params[0].kind, ParamKind::Text);
        assert_eq!(params[0].default, "v3.4.1");
        assert_eq!(params[1].kind, ParamKind::Boolean);
        assert_eq!(params[1].default, "true");
        assert_eq!(params[2].kind, ParamKind::Password);
        assert_eq!(params[2].default, "");
        assert_eq!(params[3].kind, ParamKind::Text);
    }

    #[test]
    fn build_parameters_deduplicate_property_and_actions() {
        let json = r#"{
            "property": [
                {"parameterDefinitions": [
                    {"name": "SCM_BRANCH", "type": "StringParameterDefinition"}
                ]}
            ],
            "actions": [
                {"parameterDefinitions": [
                    {"name": "SCM_BRANCH", "type": "StringParameterDefinition"},
                    {"name": "QT_VERSION", "type": "StringParameterDefinition"}
                ]}
            ]
        }"#;
        let params = parse_build_parameters(json).unwrap();
        let names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["SCM_BRANCH", "QT_VERSION"]);
    }

    #[test]
    fn build_parameter_choice_falls_back_to_first_choice() {
        // A stale default outside the choices must not be submitted.
        let json = r#"{
            "property": [
                {"parameterDefinitions": [
                    {"name": "QT_VERSION",
                     "type": "ChoiceParameterDefinition",
                     "defaultParameterValue": {"value": "gone"},
                     "choices": ["QT6_9_2", "QT5_15_2"]}
                ]}
            ]
        }"#;
        let params = parse_build_parameters(json).unwrap();
        assert_eq!(params[0].default, "QT6_9_2");
    }
}
