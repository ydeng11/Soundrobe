//! Background-tagging settings, schedule evaluation, and run history.
//!
//! The reference product runs its scraping in the background on a crontab and
//! keeps a record of what ran. The desktop app needs the same without an
//! external scheduler, so this module owns two things:
//!
//! 1. The persisted settings — whether background tagging is on, the crontab
//!    that drives it, which directories it covers, and whether it should also
//!    react to newly added files.
//! 2. The run history — what actually ran, when, and how it ended.
//!
//! Times are evaluated in the user's local wall clock, because "00:00 daily"
//! in a schedule means local midnight, not UTC midnight. The schedule itself
//! is parsed by [`super::cron`]; this module never invents a matching rule.
//!
//! Each run also carries a bounded ledger of what it did per release. The
//! reference product gets that from one database row per file; without a
//! database the run history is the durable place for it, and the bounded part
//! is what keeps "which release failed, and why" answerable after a restart
//! without letting a library-sized run grow the settings file.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

use super::cron::CronSchedule;

/// `~/.soundrobe/background-tagging.json`.
pub const SETTINGS_FILE: &str = "background-tagging.json";

/// Recent runs are kept for the Monitoring view; older ones are dropped.
const MAX_RUNS: usize = 50;

/// Per-release rows kept for one run. A run over a large library covers far
/// more releases than an operator will read, so the ledger keeps the ones that
/// need attention ahead of the rest and says that it dropped some.
const MAX_RUN_ITEMS: usize = 200;

fn default_schedule() -> String {
    "0 0 * * *".to_string()
}

/// What started a background run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    /// The crontab came due while the app was running.
    Schedule,
    /// An operator pressed "tag now".
    Manual,
    /// A watched directory gained a new file.
    Watch,
}

/// How a background run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    /// Due, but deliberately not started (another operation held the lock).
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSettings {
    pub enabled: bool,
    pub schedule: String,
    /// Album roots covered by scheduled and watched runs.
    pub directories: Vec<String>,
    /// Tag files that appear in a covered directory while the app is running.
    pub watch_enabled: bool,
    /// Only tag albums whose core tags (title/artist/album) are incomplete.
    pub skip_unchanged: bool,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            // On by default because the server already swept mounted libraries
            // for untagged releases at startup before this was configurable.
            // Defaulting to off here would silently delete that behaviour; the
            // schedule below is what makes it predictable.
            enabled: true,
            schedule: default_schedule(),
            directories: Vec::new(),
            watch_enabled: false,
            skip_unchanged: true,
        }
    }
}

/// How a release stands after the last survey or run that touched it.
///
/// One vocabulary for both the live survey map and the durable run ledger, so a
/// release cannot be described two ways depending on which surface read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseHealth {
    /// Core tags and a provider id are present.
    Healthy,
    /// Surveyed and incomplete; a run would pick it up.
    NeedsTagging,
    /// The tags could not be read.
    Unreadable,
    /// A run holds it right now.
    Tagging,
    /// A run read it but found no authoritative match.
    NeedsReview,
    /// A run failed on it.
    Failed,
}

impl ReleaseHealth {
    /// Whether attempting the release again is worth an operator's click.
    ///
    /// A healthy release is deliberately excluded: a retry that included it
    /// would re-tag the whole library instead of the part that failed.
    pub fn is_retryable(self) -> bool {
        !matches!(self, Self::Healthy | Self::Tagging)
    }
}

/// One release's outcome in a run, in the same vocabulary as the survey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseOutcome {
    pub path: String,
    pub status: ReleaseHealth,
    /// The reason code for a review, or the error for a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Which source supplied the tags, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundRun {
    pub id: String,
    pub trigger: RunTrigger,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: RunStatus,
    pub tagged: u64,
    pub failed: u64,
    pub message: String,
    /// The task registry id, so Monitoring can follow live progress.
    pub task_id: Option<String>,
    /// Per-release outcomes, oldest first. Empty for a run that covered
    /// nothing, and for history written before the ledger existed.
    #[serde(default)]
    pub items: Vec<ReleaseOutcome>,
    /// Whether `items` is a subset of what the run actually covered.
    #[serde(default)]
    pub items_truncated: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    settings: BackgroundSettings,
    #[serde(default)]
    runs: Vec<BackgroundRun>,
}

#[derive(Debug, Default)]
struct Inner {
    settings: BackgroundSettings,
    /// Newest first.
    runs: Vec<BackgroundRun>,
    /// The last schedule slot this process already evaluated, so a slot fires
    /// once even though the scheduler polls repeatedly.
    last_evaluated: Option<PrimitiveDateTime>,
    counter: u64,
}

/// Shared, cloneable background-tagging state.
#[derive(Debug, Clone)]
pub struct BackgroundState {
    path: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

impl BackgroundState {
    /// Load from `data_dir`, falling back to defaults on a missing or
    /// unreadable file. A corrupt file is reported, never silently replaced.
    pub fn init_in(data_dir: &Path) -> Self {
        let path = data_dir.join(SETTINGS_FILE);
        let persisted = match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Persisted>(&text) {
                Ok(persisted) => persisted,
                Err(error) => {
                    tracing::warn!(
                        "ignoring unreadable background-tagging settings at {}: {error}",
                        path.display()
                    );
                    Persisted::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Persisted::default(),
            Err(error) => {
                tracing::warn!(
                    "could not read background-tagging settings at {}: {error}",
                    path.display()
                );
                Persisted::default()
            }
        };

        let mut runs = persisted.runs;
        runs.truncate(MAX_RUNS);

        Self {
            path,
            inner: Arc::new(Mutex::new(Inner {
                settings: persisted.settings,
                runs,
                ..Inner::default()
            })),
        }
    }

    pub fn settings(&self) -> BackgroundSettings {
        self.lock().settings.clone()
    }

    /// Persist new settings. An unparseable crontab is rejected *before*
    /// anything is written, so the stored schedule is always runnable.
    pub fn set_settings(&self, settings: BackgroundSettings) -> Result<(), String> {
        CronSchedule::parse(&settings.schedule).map_err(|error| error.to_string())?;
        {
            let mut inner = self.lock();
            inner.settings = settings;
        }
        self.persist()
    }

    /// Newest first.
    pub fn runs(&self) -> Vec<BackgroundRun> {
        self.lock().runs.clone()
    }

    /// Record a run that is starting now and return its record.
    pub fn record_run(&self, trigger: RunTrigger, task_id: Option<String>) -> BackgroundRun {
        let mut inner = self.lock();
        inner.counter += 1;
        let run = BackgroundRun {
            id: format!("bg-run-{}-{}", inner.counter, unix_seconds()),
            trigger,
            started_at: timestamp(local_now()),
            finished_at: None,
            status: RunStatus::Running,
            tagged: 0,
            failed: 0,
            message: String::new(),
            task_id,
            items: Vec::new(),
            items_truncated: false,
        };
        inner.runs.insert(0, run.clone());
        inner.runs.truncate(MAX_RUNS);
        drop(inner);
        let _ = self.persist();
        run
    }

    /// Close out a run. Unknown ids and already-finished runs are ignored so a
    /// late task completion cannot rewrite history.
    pub fn finish_run(
        &self,
        run_id: &str,
        status: RunStatus,
        tagged: u64,
        failed: u64,
        message: impl Into<String>,
        items: Vec<ReleaseOutcome>,
    ) {
        let (items, items_truncated) = bound_items(items);
        {
            let mut inner = self.lock();
            let Some(run) = inner.runs.iter_mut().find(|run| run.id == run_id) else {
                return;
            };
            if run.status != RunStatus::Running {
                return;
            }
            run.status = status;
            run.tagged = tagged;
            run.failed = failed;
            run.message = message.into();
            run.finished_at = Some(timestamp(local_now()));
            run.items = items;
            run.items_truncated = items_truncated;
        }
        let _ = self.persist();
    }

    /// The next slot strictly after the last one already evaluated.
    ///
    /// Returns `None` unless the schedule is enabled *and* `now` has reached
    /// that slot. Reporting a slot also retires it, so an enabled schedule
    /// fires once per occurrence rather than on every poll — including when
    /// the caller decides not to run, which is why there is no separate
    /// "consume" step.
    pub fn due(&self, now: PrimitiveDateTime) -> Option<PrimitiveDateTime> {
        let mut inner = self.lock();
        if !inner.settings.enabled {
            return None;
        }
        let schedule = CronSchedule::parse(&inner.settings.schedule).ok()?;
        let current = truncate_minute(now);
        // On the first poll look one minute back, so a slot that came due just
        // before the first poll is still seen instead of silently skipped.
        let anchor = inner
            .last_evaluated
            .unwrap_or(current - Duration::minutes(1));
        let candidate = schedule.next_after(anchor)?;

        if candidate > current {
            return None;
        }
        inner.last_evaluated = Some(candidate);
        Some(candidate)
    }

    /// The next slot after `now`, without consuming anything. Used by the UI.
    /// A slot that is already due reports the one after it, because the due
    /// slot is either running or about to start.
    pub fn peek_next(&self, now: PrimitiveDateTime) -> Option<PrimitiveDateTime> {
        let inner = self.lock();
        if !inner.settings.enabled {
            return None;
        }
        let schedule = CronSchedule::parse(&inner.settings.schedule).ok()?;
        let current = truncate_minute(now);
        let anchor = inner
            .last_evaluated
            .unwrap_or(current - Duration::minutes(1));
        let candidate = schedule.next_after(anchor)?;
        if candidate > current {
            Some(candidate)
        } else {
            schedule.next_after(current)
        }
    }

    fn persist(&self) -> Result<(), String> {
        let (settings, runs) = {
            let inner = self.lock();
            (inner.settings.clone(), inner.runs.clone())
        };
        let text = serde_json::to_string_pretty(&Persisted { settings, runs })
            .map_err(|error| error.to_string())?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        // Write-then-rename so a crash mid-write cannot truncate settings.
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, text).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| {
            tracing::error!("background-tagging state mutex was poisoned; recovering");
            poisoned.into_inner()
        })
    }
}

/// Cap a run's ledger, keeping what an operator has to act on.
///
/// Successes are already counted by the run totals, so when a run covered more
/// releases than the ledger stores it is the failures and the reviews whose
/// path and reason must survive. The stable partition keeps each group in
/// encounter order, so the result is the same on every run.
fn bound_items(mut items: Vec<ReleaseOutcome>) -> (Vec<ReleaseOutcome>, bool) {
    if items.len() <= MAX_RUN_ITEMS {
        return (items, false);
    }
    items.sort_by_key(|item| !item.status.is_retryable());
    items.truncate(MAX_RUN_ITEMS);
    (items, true)
}

/// Local wall-clock now, falling back to UTC when the offset is unavailable.
pub fn local_now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

/// The wall-clock reading of an instant, which is what a crontab matches.
pub fn wall_clock(instant: OffsetDateTime) -> PrimitiveDateTime {
    PrimitiveDateTime::new(instant.date(), instant.time())
}

/// Drop seconds so a slot is compared at minute granularity, the way cron does.
fn truncate_minute(instant: PrimitiveDateTime) -> PrimitiveDateTime {
    PrimitiveDateTime::new(
        instant.date(),
        time::Time::from_hms(instant.hour(), instant.minute(), 0).unwrap_or(instant.time()),
    )
}

fn timestamp(instant: OffsetDateTime) -> String {
    instant
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("unknown"))
}

fn unix_seconds() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Date, Month, Time};

    fn dir(tag: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("soundrobe-background-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn dt(day: u8, hour: u8, minute: u8) -> PrimitiveDateTime {
        PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::September, day).unwrap(),
            Time::from_hms(hour, minute, 0).unwrap(),
        )
    }

    #[test]
    fn defaults_preserve_the_pre_existing_startup_sweep() {
        let data = dir("defaults");
        let state = BackgroundState::init_in(&data);
        let settings = state.settings();
        // Background tagging used to run unconditionally at startup; the
        // default must keep that available rather than silently disable it.
        assert!(settings.enabled);
        assert_eq!(settings.schedule, "0 0 * * *");
        assert!(settings.directories.is_empty());
        assert!(!settings.watch_enabled, "watching new files is opt-in");
        assert!(state.runs().is_empty());

        state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "*/30 * * * *".to_string(),
                directories: vec!["/music".to_string()],
                watch_enabled: true,
                skip_unchanged: false,
            })
            .unwrap();

        let reloaded = BackgroundState::init_in(&data);
        assert_eq!(reloaded.settings(), state.settings());
        assert!(reloaded.settings().enabled);
        assert_eq!(reloaded.settings().directories, vec!["/music".to_string()]);
    }

    #[test]
    fn an_invalid_schedule_is_rejected_without_overwriting_the_stored_one() {
        let data = dir("invalid");
        let state = BackgroundState::init_in(&data);
        state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "*/5 * * * *".to_string(),
                ..BackgroundSettings::default()
            })
            .unwrap();

        let error = state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "not a cron".to_string(),
                ..BackgroundSettings::default()
            })
            .expect_err("an unparseable schedule must not be saved");
        assert!(error.contains("5 fields"), "unexpected message: {error}");
        assert_eq!(state.settings().schedule, "*/5 * * * *");
        assert_eq!(
            BackgroundState::init_in(&data).settings().schedule,
            "*/5 * * * *"
        );
    }

    #[test]
    fn a_disabled_schedule_never_becomes_due() {
        let state = BackgroundState::init_in(&dir("disabled"));
        state
            .set_settings(BackgroundSettings {
                enabled: false,
                schedule: "*/1 * * * *".to_string(),
                ..BackgroundSettings::default()
            })
            .unwrap();
        assert_eq!(state.due(dt(18, 13, 45)), None);
        assert_eq!(state.peek_next(dt(18, 13, 45)), None);
    }

    #[test]
    fn an_enabled_schedule_fires_once_per_slot() {
        let state = BackgroundState::init_in(&dir("due"));
        state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "0 0 * * *".to_string(),
                ..BackgroundSettings::default()
            })
            .unwrap();

        // A poll inside the 00:00 minute reports that slot.
        assert_eq!(state.due(dt(19, 0, 0)), Some(dt(19, 0, 0)));
        // The same slot must not fire again...
        assert_eq!(state.due(dt(19, 0, 0)), None);
        assert_eq!(state.due(dt(19, 23, 59)), None);
        // ...but the next day does.
        assert_eq!(state.due(dt(20, 0, 0)), Some(dt(20, 0, 0)));
    }

    #[test]
    fn a_slot_that_a_delayed_poll_missed_still_fires_late() {
        // Celery Beat-style behaviour: a delayed poll catches up instead of
        // dropping the occurrence, but only one slot at a time.
        let state = BackgroundState::init_in(&dir("catchup"));
        state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "*/5 * * * *".to_string(),
                ..BackgroundSettings::default()
            })
            .unwrap();

        assert_eq!(state.due(dt(18, 13, 45)), Some(dt(18, 13, 45)));
        // The poll that should have run at 13:50 arrives at 13:52.
        assert_eq!(state.due(dt(18, 13, 52)), Some(dt(18, 13, 50)));
        // Only the one missed slot fires; 13:55 is still in the future.
        assert_eq!(state.due(dt(18, 13, 52)), None);
        assert_eq!(state.due(dt(18, 13, 55)), Some(dt(18, 13, 55)));
    }

    #[test]
    fn a_poll_before_the_first_slot_waits() {
        let state = BackgroundState::init_in(&dir("wait"));
        state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "0 3 * * *".to_string(),
                ..BackgroundSettings::default()
            })
            .unwrap();
        assert_eq!(state.due(dt(18, 13, 45)), None);
        assert_eq!(state.peek_next(dt(18, 13, 45)), Some(dt(19, 3, 0)));
        // Peeking must not consume the slot.
        assert_eq!(state.peek_next(dt(18, 13, 45)), Some(dt(19, 3, 0)));
    }

    #[test]
    fn runs_are_recorded_newest_first_and_closed_once() {
        let data = dir("runs");
        let state = BackgroundState::init_in(&data);
        let first = state.record_run(RunTrigger::Schedule, Some("auto-tag-1".to_string()));
        let second = state.record_run(RunTrigger::Manual, None);

        let runs = state.runs();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, second.id, "newest run comes first");
        assert_eq!(runs[0].status, RunStatus::Running);
        assert_eq!(runs[1].id, first.id);

        state.finish_run(
            &first.id,
            RunStatus::Completed,
            4,
            1,
            "tagged 4",
            Vec::new(),
        );

        let runs = state.runs();
        let closed = runs.iter().find(|run| run.id == first.id).unwrap();
        assert_eq!(closed.status, RunStatus::Completed);
        assert_eq!(closed.tagged, 4);
        assert_eq!(closed.failed, 1);
        assert_eq!(closed.message, "tagged 4");
        assert!(closed.finished_at.is_some());
        assert!(runs[0].finished_at.is_none(), "the running run stays open");

        // A late completion must not rewrite history, and history survives reload.
        state.finish_run(&first.id, RunStatus::Failed, 0, 0, "late", Vec::new());
        assert_eq!(
            state
                .runs()
                .iter()
                .find(|run| run.id == first.id)
                .unwrap()
                .status,
            RunStatus::Completed
        );
        state.finish_run(
            "missing-run",
            RunStatus::Failed,
            0,
            0,
            "unknown",
            Vec::new(),
        );
        let reloaded = BackgroundState::init_in(&data);
        assert_eq!(reloaded.runs().len(), 2);
        assert_eq!(reloaded.runs()[0].id, second.id);
    }

    #[test]
    fn a_run_records_which_release_each_outcome_came_from() {
        // The reference product answers "which file failed, and why" from a row
        // per file. Without a database the run history has to hold it, and it
        // has to survive a restart - that durability is the whole point.
        let data = dir("ledger");
        let state = BackgroundState::init_in(&data);
        let run = state.record_run(
            RunTrigger::Manual,
            Some("background-auto-tag-1".to_string()),
        );
        state.finish_run(
            &run.id,
            RunStatus::Completed,
            1,
            1,
            "Tagged 1, 1 failed",
            vec![
                ReleaseOutcome {
                    path: "/music/Album A".to_string(),
                    status: ReleaseHealth::Healthy,
                    reason: None,
                    authority: Some("discogs".to_string()),
                },
                ReleaseOutcome {
                    path: "/music/Album B".to_string(),
                    status: ReleaseHealth::Failed,
                    reason: Some("permission denied".to_string()),
                    authority: None,
                },
            ],
        );

        let reloaded = BackgroundState::init_in(&data);
        let run = &reloaded.runs()[0];
        assert_eq!(run.items.len(), 2);
        assert_eq!(run.items[0].path, "/music/Album A");
        assert_eq!(run.items[0].status, ReleaseHealth::Healthy);
        assert_eq!(run.items[0].authority.as_deref(), Some("discogs"));
        assert_eq!(run.items[1].status, ReleaseHealth::Failed);
        assert_eq!(run.items[1].reason.as_deref(), Some("permission denied"));
        assert!(!run.items_truncated);
    }

    #[test]
    fn a_large_run_keeps_the_releases_that_need_attention() {
        let state = BackgroundState::init_in(&dir("bounded-items"));
        let run = state.record_run(RunTrigger::Schedule, None);
        let mut items = (0..MAX_RUN_ITEMS)
            .map(|index| ReleaseOutcome {
                path: format!("/music/ok-{index}"),
                status: ReleaseHealth::Healthy,
                reason: None,
                authority: None,
            })
            .collect::<Vec<_>>();
        items.push(ReleaseOutcome {
            path: "/music/broken".to_string(),
            status: ReleaseHealth::Failed,
            reason: Some("boom".to_string()),
            authority: None,
        });
        state.finish_run(
            &run.id,
            RunStatus::Failed,
            MAX_RUN_ITEMS as u64,
            1,
            "one failed",
            items,
        );

        let stored = &state.runs()[0];
        assert_eq!(stored.items.len(), MAX_RUN_ITEMS);
        assert!(stored.items_truncated);
        // The failure is the row an operator has to act on, so it is what
        // survives the cap; the dropped rows are successes, which the run
        // totals already account for.
        assert_eq!(stored.items[0].path, "/music/broken");
        assert!(
            !stored.items.iter().any(|item| item.path == "/music/ok-199"),
            "the ledger must drop successes, not failures"
        );
    }

    #[test]
    fn run_history_written_before_the_ledger_still_loads() {
        // An existing install has run history and no `items`. Failing to read it
        // would silently discard the operator's history on upgrade.
        let data = dir("legacy-runs");
        fs::write(
            data.join(SETTINGS_FILE),
            r#"{"settings":{"enabled":true,"schedule":"0 0 * * *","directories":[],"watchEnabled":false,"skipUnchanged":true},"runs":[{"id":"bg-run-1-0","trigger":"manual","startedAt":"2026-09-19T00:00:00Z","finishedAt":"2026-09-19T00:01:00Z","status":"completed","tagged":2,"failed":0,"message":"Tagged 2","taskId":null}]}"#,
        )
        .unwrap();

        let state = BackgroundState::init_in(&data);
        assert_eq!(state.runs().len(), 1);
        assert!(state.runs()[0].items.is_empty());
        assert!(!state.runs()[0].items_truncated);
    }

    #[test]
    fn only_outcomes_worth_repeating_are_retryable() {
        // A retry that included healthy releases would re-tag the whole library
        // instead of the part that failed.
        assert!(ReleaseHealth::Failed.is_retryable());
        assert!(ReleaseHealth::Unreadable.is_retryable());
        assert!(ReleaseHealth::NeedsReview.is_retryable());
        assert!(ReleaseHealth::NeedsTagging.is_retryable());
        assert!(!ReleaseHealth::Healthy.is_retryable());
        assert!(!ReleaseHealth::Tagging.is_retryable());
    }

    #[test]
    fn history_is_bounded() {
        let state = BackgroundState::init_in(&dir("bounded"));
        for _ in 0..(MAX_RUNS + 5) {
            state.record_run(RunTrigger::Watch, None);
        }
        assert_eq!(state.runs().len(), MAX_RUNS);
    }

    #[test]
    fn a_corrupt_settings_file_falls_back_to_defaults_instead_of_panicking() {
        let data = dir("corrupt");
        fs::write(data.join(SETTINGS_FILE), "{ not json").unwrap();
        let state = BackgroundState::init_in(&data);
        assert_eq!(state.settings(), BackgroundSettings::default());
        assert!(state.runs().is_empty());
    }

    #[test]
    fn a_reported_slot_does_not_repeat_even_when_the_run_never_starts() {
        let state = BackgroundState::init_in(&dir("skipped"));
        state
            .set_settings(BackgroundSettings {
                enabled: true,
                schedule: "*/5 * * * *".to_string(),
                ..BackgroundSettings::default()
            })
            .unwrap();

        // Reporting retires the slot, so a caller that declines to run (say,
        // another operation holds the lock) cannot get it reported twice.
        let slot = state.due(dt(18, 13, 45)).unwrap();
        assert_eq!(slot, dt(18, 13, 45));
        assert_eq!(state.due(dt(18, 13, 45)), None);
        assert_eq!(state.peek_next(dt(18, 13, 45)), Some(dt(18, 13, 50)));
    }
}
