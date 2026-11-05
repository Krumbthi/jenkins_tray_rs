# Feature: "Build periodically with parameters"

## Goal

Allow a Jenkins job to be triggered on a cron schedule with build parameters, mirroring
Jenkins' "Build periodically with parameters" trigger syntax:

```
H 23 * * * %BUILD_TYPE=DBG_REL;QT_LIB_VERSION=SHARED;BUILD_QTWEBENGINE=false;CLEAN_BUILD=true;CLEAN_REPO=true;GENERATE_SBOM=true
```

The part before `%` is a Jenkins cron expression (with `H` hash tokens). The part after
`%` is a semicolon-separated list of `KEY=VALUE` build parameters that are posted to
`buildWithParameters`.

## Implementation

1. **Parser** (`jenkins_client::parse_periodic_trigger`)
   - Splits the spec on the first `%`: left side cron, right side optional
     `KEY=VALUE` parameters separated by `;`.
   - Parameters are form-encoded by `trigger_build_parameters` (same call as a
     manual parameterized build).

2. **Scheduler** (`projects_updater::MinuteScheduler`)
   - Each project carries an optional `periodic_trigger: Option<String>` in the
     config JSON (`periodicTrigger`, serde default so old configs keep working).
   - A dedicated tokio task wakes on every minute boundary and checks the five
     cron fields, including `H`, `H/step`, `H(a-b)`, ranges, lists, steps, names
     and the `@hourly`/`@daily`/… aliases. It calls `trigger_build_parameters`
     for every matching project.
   - `H` offsets are derived from a stable hash of the job name, so they stay
     the same across restarts. This does not reproduce Jenkins' exact hash
     algorithm; only the spreading/stability property matters here.
   - Scheduling runs only while the tray application is running and does not
     catch up on missed runs (matching the original recommendation below).
   - Duplicate triggers within the same minute are suppressed.

3. **Jenkins client**
   - `trigger_build_parameters(client, server, project, params)` posts to
     `buildWithParameters` with a form-encoded body.

4. **UI**
   - The schedule controls live at the bottom of the existing "Build with
     parameters" dialog (enable checkbox + Jenkins cron field); there is no
     separate context-menu entry. The parameter list above is reused for both
     the manual and the periodic runs.
   - The dialog validates the cron expression before saving it. Password values
     are never persisted, so they are omitted from the saved schedule (a warning
     is logged).

5. **Notifications / logging**
   - Every scheduled trigger and failure is logged. Tray notifications are not
     raised for these runs.

## Open questions (still open)

- Should the dialog show a "next run" preview?
- Should jobs with a periodic trigger get a small grid indicator?
- `H` hashing duplicates the stability property but not Jenkins' exact
  algorithm; acceptable because these triggers are client-side only.
