//! A job's schedule: a fixed interval or a cron expression, parsed from the raw string a
//! `#[job(..)]` attribute carries.
//!
//! Parsing is deferred to scheduler startup (the descriptor stores only the raw literal), so
//! an invalid schedule surfaces as a startup error rather than silently never firing.

use std::str::FromStr;
use std::time::{Duration, SystemTime};

use chrono::{Local, Utc};
use croner::Cron;

#[cfg(test)]
mod tests;

/// Which flavour of schedule a raw string is: an [`every`](ScheduleKind::Interval) duration
/// or a [`cron`](ScheduleKind::Cron) expression. Emitted by the `#[job]` macro from whether
/// the attribute used `every = ..` or `cron = ..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    /// A fixed `every = ".."` interval, run on the monotonic clock.
    ///
    /// The value is a `humantime` duration (`30s`, `5m`, `1h 30m`, `500ms`, and even
    /// `2 months`). Note that `months`/`years` are *fixed approximations* (30.44 / 365.25
    /// days), so an interval is a constant wall-time gap — it does not align to the calendar.
    /// For a calendar-aware cadence (e.g. the 1st of every other month), use a
    /// [`Cron`](ScheduleKind::Cron) schedule such as `"0 0 1 */2 *"` instead.
    Interval,
    /// A `cron = ".."` expression (or an `@`-nickname), run on the wall clock.
    Cron,
}

/// A parsed schedule, ready for the scheduler to drive.
pub enum Schedule {
    /// Run once per fixed period.
    Every(Duration),
    /// Run at each occurrence of a cron expression.
    Cron(Box<Cron>),
}

/// A schedule string that could not be parsed.
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    /// An `every = ".."` value `humantime` could not parse (`30s`, `5m`, `1h 30m`, `500ms`).
    #[error("invalid interval '{expr}': {source}")]
    Interval {
        expr: String,
        source: humantime::DurationError,
    },

    /// A zero interval, which would never make progress (and panics `tokio::time::interval`).
    #[error("invalid interval '{expr}': the period must be greater than zero")]
    ZeroInterval { expr: String },

    /// A `cron = ".."` expression the cron parser rejected. Cron nicknames (`@hourly`, …) are
    /// accepted natively by the parser.
    #[error("invalid cron expression '{expr}': {source}")]
    Cron {
        expr: String,
        source: croner::errors::CronError,
    },
}

impl Schedule {
    /// A fixed-interval schedule from a [`Duration`] — the constructor for a runtime job whose
    /// period is already computed (e.g. read from a database row).
    pub fn every(period: Duration) -> Self {
        Schedule::Every(period)
    }

    /// A fixed-interval schedule from a human string (`"30s"`, `"1h 30m"`), for runtime jobs.
    pub fn interval(expr: &str) -> Result<Self, ScheduleError> {
        Self::parse(ScheduleKind::Interval, expr)
    }

    /// A cron schedule from an expression or `@`-nickname, for runtime jobs.
    pub fn cron(expr: &str) -> Result<Self, ScheduleError> {
        Self::parse(ScheduleKind::Cron, expr)
    }

    /// Parses a raw schedule string according to its [`ScheduleKind`]. Interval strings go
    /// through [`humantime`]; cron strings (including `@`-nicknames) through [`croner`].
    pub fn parse(kind: ScheduleKind, raw: &str) -> Result<Self, ScheduleError> {
        let expr = raw.trim();

        match kind {
            ScheduleKind::Interval => {
                let period =
                    humantime::parse_duration(expr).map_err(|source| ScheduleError::Interval {
                        expr: raw.to_string(),
                        source,
                    })?;

                if period.is_zero() {
                    return Err(ScheduleError::ZeroInterval {
                        expr: raw.to_string(),
                    });
                }

                Ok(Schedule::Every(period))
            }

            ScheduleKind::Cron => Cron::from_str(expr)
                .map(|cron| Schedule::Cron(Box::new(cron)))
                .map_err(|source| ScheduleError::Cron {
                    expr: raw.to_string(),
                    source,
                }),
        }
    }

    /// A backend-agnostic, `Clone`able description of this schedule, for introspection
    /// (`JobInfo::schedule`). A cron schedule reports its original pattern string; an
    /// interval reports its [`Duration`].
    pub fn describe(&self) -> ScheduleInfo {
        match self {
            Schedule::Every(period) => ScheduleInfo::Every(*period),
            Schedule::Cron(cron) => ScheduleInfo::Cron(cron.pattern.as_str().to_string()),
        }
    }

    /// The next wall-clock occurrence of a [`Cron`](Schedule::Cron) schedule after `after`,
    /// in `timezone`. Returns `None` for an [`Every`](Schedule::Every) schedule (whose cadence
    /// is driven by the monotonic timer, not the wall clock) or when the cron has no further
    /// occurrence.
    pub fn next_cron_occurrence(
        &self,
        after: SystemTime,
        timezone: JobTimezone,
    ) -> Option<SystemTime> {
        let Schedule::Cron(cron) = self else {
            return None;
        };

        let after: chrono::DateTime<Utc> = after.into();

        let next = match timezone {
            JobTimezone::Utc => cron.find_next_occurrence(&after, false).ok()?.into(),

            JobTimezone::Local => {
                let local = after.with_timezone(&Local);

                cron.find_next_occurrence(&local, false)
                    .ok()?
                    .with_timezone(&Utc)
                    .into()
            }
        };

        Some(next)
    }
}

/// A `Clone`able, backend-agnostic description of a job's schedule, surfaced through
/// [`JobInfo::schedule`](crate::JobInfo). Unlike [`Schedule`], it holds no parsed cron
/// state, so it can be freely cloned into introspection results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleInfo {
    /// A fixed interval (`every = ".."`).
    Every(Duration),
    /// A cron expression, reported as its original pattern string.
    Cron(String),
}

/// The wall clock a [`cron`](Schedule::Cron) schedule is computed against.
///
/// Interval schedules ignore this — they run on the monotonic clock. Defaults to
/// [`Utc`](JobTimezone::Utc), preserving the historical behaviour of computing cron
/// occurrences from UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobTimezone {
    /// Compute cron occurrences from wall-clock UTC.
    #[default]
    Utc,
    /// Compute cron occurrences from the host's local timezone.
    Local,
}

/// What the scheduler does when a job's schedule fires while a previous run is still active.
///
/// The default is [`Skip`](OverlapPolicy::Skip), which preserves the original scheduler
/// behaviour: a run is never started while another is in flight, so a body that outlasts its
/// period simply defers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlapPolicy {
    /// Skip the firing if a run is already active (the historical default).
    #[default]
    Skip,
    /// Skip while active, but remember that one firing was missed and run it once the active
    /// run finishes.
    QueueOne,
    /// Always start a new run, allowing runs to overlap.
    Allow,
    /// Cancel the active run's token, then start the new run.
    CancelPrevious,
}

/// Per-job execution policy layered on top of the [`Schedule`]: how a job runs, not when.
///
/// [`Default`] preserves the original behaviour exactly — no run on startup, no timeout, no
/// jitter, non-overlapping runs ([`OverlapPolicy::Skip`]), and cron computed from
/// [`Utc`](JobTimezone::Utc).
#[derive(Debug, Clone, Default)]
pub struct JobOptions {
    /// Run once immediately when the job is scheduled, before waiting for the first occurrence.
    pub run_on_startup: bool,
    /// Abort a run that exceeds this duration, recording it as timed out.
    pub timeout: Option<Duration>,
    /// A random delay of up to this duration added before each scheduled run, to spread load.
    pub jitter: Option<Duration>,
    /// How to handle a firing that overlaps an active run.
    pub overlap: OverlapPolicy,
    /// A hard ceiling on a single run's wall-clock time. Enforced identically to
    /// [`timeout`](Self::timeout); when both are set the smaller wins.
    pub max_runtime: Option<Duration>,
    /// The timezone a [`cron`](Schedule::Cron) schedule is computed against.
    pub timezone: Option<JobTimezone>,
}

impl JobOptions {
    /// The effective per-run deadline: the smaller of [`timeout`](Self::timeout) and
    /// [`max_runtime`](Self::max_runtime), or `None` when neither is set.
    pub fn deadline(&self) -> Option<Duration> {
        match (self.timeout, self.max_runtime) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// The configured timezone, or the [`Utc`](JobTimezone::Utc) default.
    pub fn timezone(&self) -> JobTimezone {
        self.timezone.unwrap_or_default()
    }
}
