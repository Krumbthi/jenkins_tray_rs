//! Domain model for Jenkins Tray.
//!
//! Mirrors C# `JenkinsTray.Entities` namespace with data structures
//! for Jenkins server, projects, build status, and configuration.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::cmp::Ordering;

/// Jenkins server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Server {
    /// Jenkins instance URL (e.g. https://ci.example.com)
    pub url: String,
    /// Display name for the server (shown in UI)
    pub display_name: String,
    /// Optional username for authentication
    pub username: Option<String>,
    /// Optional password for authentication (kept in memory only)
    pub password: Option<String>,
    /// If true, ignore untrusted SSL certificates
    pub ignore_untrusted_certificate: bool,
    /// Project list owned by this server
    pub projects: Vec<Project>,
}

/// A Jenkins job/project.
///
/// Mirrors the C# `Project` entity: only `name`/`url`/`displayName`/
/// `acknowledged`/`token`/`causetext` are persisted (the original declares
/// `[JsonProperty]` only on those); `queue` and `all_build_details` are
/// runtime state refreshed by the update service and are never written to
/// disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Project {
    /// Project/folder name on the Jenkins server
    pub name: String,
    /// URL to the project on the Jenkins server
    pub url: String,
    /// Associated server (for grouping in UI)
    #[serde(skip)]
    pub server: Option<String>,
    /// Human-readable display name (original: Project.DisplayName, JSON
    /// "displayName" - persisted by the original as well)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Queue item info if a build is pending (original: Project.Queue, runtime
    /// only)
    #[serde(skip_serializing, default)]
    pub queue: Option<QueueItem>,
    /// Live build state fetched from Jenkins (original: Project.AllBuildDetails,
    /// runtime only)
    #[serde(skip_serializing, default)]
    pub all_build_details: Option<AllBuildDetails>,
    /// Whether the project's status was acknowledged in the tray
    /// (original: Project.IsAcknowledged, JSON "acknowledged")
    #[serde(default, rename = "acknowledged")]
    pub is_acknowledged: bool,
    /// Authentication token used to trigger builds
    /// (original: Project.AuthenticationToken, JSON "token")
    #[serde(default, rename = "token", skip_serializing_if = "Option::is_none")]
    pub authentication_token: Option<String>,
    /// Cause text passed when triggering a build
    /// (original: Project.CauseText, JSON "causetext")
    #[serde(default, rename = "causetext", skip_serializing_if = "Option::is_none")]
    pub cause_text: Option<String>,
}

impl Project {
    /// The project's current status
    /// (original: Project.Status - `UNKNOWN_BUILD_STATUS` without details).
    pub fn status(&self) -> BuildStatus {
        self.all_build_details
            .as_ref()
            .map(|details| details.status.clone())
            .unwrap_or_else(BuildStatus::unknown)
    }

    /// Whether any build exists yet (original: `JenkinsService.HasBuild`).
    pub fn has_build(&self) -> bool {
        let Some(details) = self.all_build_details.as_ref() else {
            return false;
        };
        if details.last_completed_build.is_some() {
            return true;
        }
        details.status.is_in_progress
    }
}

/// Build status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BuildStatus {
    /// The build status enum value
    pub value: BuildStatusEnum,
    /// Whether the build is currently in progress (animated indicator)
    pub is_in_progress: bool,
    /// Whether the build is stuck
    pub is_stuck: bool,
}

impl BuildStatus {
    /// The unknown status (original: BuildStatus.UNKNOWN_BUILD_STATUS).
    pub fn unknown() -> Self {
        BuildStatus {
            value: BuildStatusEnum::Unknown,
            is_in_progress: false,
            is_stuck: false,
        }
    }

    /// Resource key of the status description (original:
    /// `BuildStatus.Key` with the `BuildStatus_` prefix the callers add:
    /// `Value.ToString()` plus `_BuildInProgress` / `_Stuck` suffixes).
    pub fn resource_key(&self) -> String {
        let name = match self.value {
            BuildStatusEnum::Unknown => "Unknown",
            BuildStatusEnum::Disabled => "Disabled",
            BuildStatusEnum::Aborted => "Aborted",
            BuildStatusEnum::Successful => "Successful",
            BuildStatusEnum::Indeterminate => "Indeterminate",
            BuildStatusEnum::Unstable => "Unstable",
            BuildStatusEnum::Failed => "Failed",
        };
        let mut key = format!("BuildStatus_{name}");
        if self.is_in_progress {
            key.push_str("_BuildInProgress");
        }
        if self.is_stuck {
            key.push_str("_Stuck");
        }
        key
    }

    /// Create from Jenkins color + stuck state
    /// (original: JenkinsService.GetStatus).
    pub fn from_jenkins_color(color: &str, stuck: bool) -> Self {
        let col = color.to_lowercase();
        let value = if col.starts_with("grey") {
            BuildStatusEnum::Indeterminate
        } else if col.starts_with("blue") || col.starts_with("green") {
            BuildStatusEnum::Successful
        } else if col.starts_with("yellow") {
            BuildStatusEnum::Unstable
        } else if col.starts_with("red") {
            BuildStatusEnum::Failed
        } else if col.starts_with("aborted") {
            BuildStatusEnum::Aborted
        } else if col.starts_with("disabled") {
            BuildStatusEnum::Disabled
        } else {
            BuildStatusEnum::Unknown
        };
        let in_progress_flag = col.ends_with("_anime");
        BuildStatus {
            value,
            is_in_progress: in_progress_flag,
            is_stuck: stuck,
        }
    }

    /// Parse a Jenkins build `result` XML value
    /// (original: BuildStatus.StringToBuildStatus).
    pub fn from_jenkins_result(result: &str) -> BuildStatusEnum {
        match result.to_lowercase().as_str() {
            "success" => BuildStatusEnum::Successful,
            "failure" => BuildStatusEnum::Failed,
            "unstable" => BuildStatusEnum::Unstable,
            "aborted" => BuildStatusEnum::Aborted,
            _ => BuildStatusEnum::Unknown,
        }
    }

    /// Whether this status is more severe than `than`
    /// (original: BuildStatusUtils.IsWorse).
    pub fn is_worse(&self, than: &BuildStatus) -> bool {
        self.value > than.value
    }

    /// Failed or unstable (original: AllServersStatus.TreatAsFailure - the
    /// original bucket logic does not consult the unstable setting).
    pub fn treat_as_failure(&self) -> bool {
        matches!(self.value, BuildStatusEnum::Failed | BuildStatusEnum::Unstable)
    }

    /// Get the display key (e.g. "success", "failed_BuildInProgress", "aborted_Stuck").
    pub fn key(&self) -> String {
        let mut res = self.value.to_string();
        if self.is_in_progress {
            res += "_BuildInProgress";
        }
        if self.is_stuck {
            res += "_Stuck";
        }
        res
    }
}

/// Jenkins build status enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatusEnum {
    #[default]
    Unknown,
    Disabled,
    Aborted,
    Successful,
    Indeterminate,
    Unstable,
    Failed,
}

impl std::fmt::Display for BuildStatusEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildStatusEnum::Unknown => write!(f, "unknown"),
            BuildStatusEnum::Disabled => write!(f, "disabled"),
            BuildStatusEnum::Aborted => write!(f, "aborted"),
            BuildStatusEnum::Successful => write!(f, "success"),
            BuildStatusEnum::Indeterminate => write!(f, "indeterminate"),
            BuildStatusEnum::Unstable => write!(f, "unstable"),
            BuildStatusEnum::Failed => write!(f, "failed"),
        }
    }
}

impl BuildStatusEnum {
    /// Total-order rank for the port of the original's hand-written `cmp`
    /// (used by `is_worse`, the tray icon's "worst status" reduction and the
    /// `>= Indeterminate` acknowledgement threshold).
    ///
    /// The original match was not a total order: `(Unstable, _) => Less`
    /// contradicted `(Successful, _) => Less` and `(Unknown, _) => Less`, so
    /// `Unstable` compared `Less` in *both* directions against `Successful`
    /// and `Unknown`. Every other pair was consistent; this rank keeps those
    /// relations and resolves the contradictions by dropping the
    /// `unstable < everything` claim, which is also the only reading that
    /// agrees with `treat_as_failure` (unstable builds are failures).
    ///
    /// Ascending: successful < unknown < unstable < disabled < aborted <
    /// indeterminate < failed.
    fn severity(self) -> u8 {
        match self {
            BuildStatusEnum::Successful => 0,
            BuildStatusEnum::Unknown => 1,
            BuildStatusEnum::Unstable => 2,
            BuildStatusEnum::Disabled => 3,
            BuildStatusEnum::Aborted => 4,
            BuildStatusEnum::Indeterminate => 5,
            BuildStatusEnum::Failed => 6,
        }
    }
}

impl Ord for BuildStatusEnum {
    fn cmp(&self, other: &Self) -> Ordering {
        self.severity().cmp(&other.severity())
    }
}

impl PartialOrd for BuildStatusEnum {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Queue item info for pending builds (original: `Project.QueueItem`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct QueueItem {
    /// Whether the project has a queued build (original: InQueue)
    #[serde(default)]
    pub in_queue: bool,
    /// Queue item ID
    pub id: Option<u64>,
    /// Whether the item is stuck in queue
    pub stuck: Option<bool>,
    /// Why the item is in queue
    pub why: Option<String>,
    /// Time the item entered queue (milliseconds since epoch)
    pub in_queue_since: Option<i64>,
}

/// Detailed build information (original: `BuildDetails`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BuildDetails {
    /// Build number
    pub number: u32,
    /// Display name (e.g. "#123 MyJob")
    pub display_name: String,
    /// Build start time, milliseconds since epoch (original: Time)
    pub time: i64,
    /// Estimated duration in milliseconds (original: EstimatedDuration)
    pub estimated_duration: Option<i64>,
    /// Actual duration in milliseconds (original: Duration)
    pub duration: Option<i64>,
    /// Build result (original: Result - parsed with StringToBuildStatus)
    #[serde(default)]
    pub result: BuildStatusEnum,
    /// Users who caused the build (original: Users - "culprits")
    pub users: HashSet<String>,
    /// Build causes (original: BuildDetails.Causes)
    #[serde(skip_serializing, default)]
    pub causes: Option<BuildCauses>,
    /// Claim plugin details of this build (original: ClaimDetails)
    #[serde(skip_serializing, default)]
    pub claim_details: Option<ClaimDetails>,
}

/// Aggregated build details for a project (original: `AllBuildDetails`).
/// Runtime state - only reached through `Project.all_build_details`, which is
/// not serialized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AllBuildDetails {
    /// Overall status parsed from the job color (original: Status)
    pub status: BuildStatus,
    /// Status of the previous poll cycle (original: PreviousStatus)
    #[serde(default)]
    pub previous_status: Option<BuildStatus>,
    /// The most recent build (original: LastBuild)
    pub last_build: Option<BuildDetails>,
    /// The most recent completed build (original: LastCompletedBuild)
    pub last_completed_build: Option<BuildDetails>,
    /// The most recent successful build (original: LastSuccessfulBuild)
    pub last_successful_build: Option<BuildDetails>,
    /// The most recent failed build (original: LastFailedBuild)
    pub last_failed_build: Option<BuildDetails>,
}

/// Claim plugin details of a claimed build (original: `ClaimDetails`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClaimDetails {
    /// Who assigned the claim (original: Assignor)
    pub assignor: String,
    /// Who the claim is assigned to (original: Assignee)
    pub assignee: String,
    /// Claim reason (original: Reason)
    pub reason: String,
}

/// Classification of a build cause (original: `BuildCauseEnum`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum BuildCauseEnum {
    #[default]
    Unknown,
    /// Name kept verbatim from the original C# enum (`BuildCauseEnum.SCM`).
    #[allow(clippy::upper_case_acronyms)]
    SCM,
    Timer,
    User,
    UpstreamProject,
    RemoteHost,
}

/// One build cause (original: `BuildCause`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BuildCause {
    /// Classified cause
    pub cause: BuildCauseEnum,
    /// Jenkins `shortDescription`
    pub short_description: String,
    /// User/upstream project/remote host that started the build
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starter: Option<String>,
    /// Jenkins user id (User causes only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// The set of causes of a build (original: `BuildCauses`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BuildCauses {
    /// All causes in Jenkins order
    pub causes: Vec<BuildCause>,
    /// `None` without causes, otherwise whether all causes are equal
    /// (original: HasUniqueCauses)
    pub has_unique_causes: Option<bool>,
}

/// Runtime state fetched from Jenkins for one project by one update cycle.
///
/// Produced by `jenkins_client::update_project` and applied to the
/// configuration in memory (never persisted), mirroring how the original
/// mutates its shared `Project` objects.
#[derive(Debug, Clone, Default)]
pub struct ProjectState {
    /// Job `displayName` (always overwritten - the original assigns it
    /// unconditionally)
    pub display_name: Option<String>,
    /// Whether the job currently has a queued item (always overwritten)
    pub in_queue: bool,
    /// Queue item id, `None` to leave the stored value untouched
    /// (the original only overwrites when the XML contains a value)
    pub queue_id: Option<u64>,
    /// Queue `why`, `None` to leave untouched
    pub queue_why: Option<String>,
    /// Queue `stuck`, `None` to leave untouched
    pub queue_stuck: Option<bool>,
    /// Queue `inQueueSince` (epoch millis), `None` to leave untouched
    pub in_queue_since: Option<i64>,
    /// Fresh details, `None` when a build-details download failed (the
    /// original then keeps the previous `AllBuildDetails` untouched)
    pub details: Option<AllBuildDetails>,
}

/// Global application configuration.
///
/// **Documented deviation:** the original serializes this as camelCase
/// (`notificationSettings`, `generalSettings`, server `displayName`,
/// credentials as a `["user", "base64"]` array, sound paths as
/// `failedSoundPath`/`succeededSoundPath`/`stillFailingSoundPath`/
/// `fixedSoundPath`); this port keeps its own snake_case format with flat
/// `username`/`password` and a `sound_paths` map, so config files are *not*
/// interchangeable with the original application (user decision).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Configuration {
    /// List of configured Jenkins servers
    pub servers: Vec<Server>,
    /// Global notification settings
    pub notification_settings: NotificationSettings,
    /// General application settings
    pub general_settings: GeneralSettings,
    /// Last submitted parameter values per job URL, so the "Build with
    /// parameters" dialog can pre-fill the last choices (new feature; the
    /// original has no parameter dialog). Password parameters are never
    /// stored.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub last_build_parameters: HashMap<String, HashMap<String, String>>,
}

/// Global notification settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct NotificationSettings {
    /// If true, show balloon notifications in the tray
    pub balloon_notifications: bool,
    /// If true, play sound notifications
    pub sound_notifications: bool,
    /// If true, treat unstable builds as failures
    pub treat_unstable_as_failed: bool,
    /// Paths to sound files for different statuses (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_paths: Option<HashMap<String, String>>,
}

/// General application settings.
///
/// Defaults mirror the original `GeneralSettings()` constructor
/// (`DEFAULT_TIME_BETWEEN_UPDATES = 15`, `CheckForUpdates = true`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GeneralSettings {
    /// Refresh interval in seconds (original default 15)
    pub refresh_interval_in_seconds: u64,
    /// If true, update the main window icon on status change
    pub update_main_window_icon: bool,
    /// If true, integrate with Jenkins Claim Plugin
    pub integrate_with_claim_plugin: bool,
    /// If true, show project display names in the main UI
    pub show_project_display_name: bool,
    /// If true, check for application updates at startup and hourly
    pub check_for_updates: bool,
    /// If true, register the application in the Windows `Run` registry key
    /// so it starts automatically when the user logs in. Ignored on non-Windows
    /// platforms.
    pub start_with_windows: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            refresh_interval_in_seconds: 15,
            update_main_window_icon: false,
            integrate_with_claim_plugin: false,
            show_project_display_name: false,
            check_for_updates: true,
            start_with_windows: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_ordering_is_total_and_keeps_the_thresholds() {
        use BuildStatusEnum::*;

        let ascending = [
            Successful,
            Unknown,
            Unstable,
            Disabled,
            Aborted,
            Indeterminate,
            Failed,
        ];
        for pair in ascending.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} should sort before {:?}",
                pair[0],
                pair[1]
            );
        }

        // Thresholds of the grid/tray code: "worse than successful" (the
        // details/tooltip counts and the regression check), "at least
        // indeterminate" (ack status eligibility) and "failed".
        assert!(Unknown > Successful);
        assert!(Unstable > Successful);
        assert!(Unknown < Indeterminate);
        assert!(Failed > Indeterminate);
        assert!(Indeterminate > Aborted);
    }
}
