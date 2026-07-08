//! A job's schedule: a fixed interval or a cron expression, parsed from the raw string a
//! `#[job(..)]` attribute carries.
//!
//! Parsing is deferred to scheduler startup (the descriptor stores only the raw literal), so
//! an invalid schedule surfaces as a startup error rather than silently never firing.

use std::str::FromStr;
use std::time::Duration;

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
            ScheduleKind::Interval => humantime::parse_duration(expr)
                .map(Schedule::Every)
                .map_err(|source| ScheduleError::Interval {
                    expr: raw.to_string(),
                    source,
                }),

            ScheduleKind::Cron => Cron::from_str(expr)
                .map(|cron| Schedule::Cron(Box::new(cron)))
                .map_err(|source| ScheduleError::Cron {
                    expr: raw.to_string(),
                    source,
                }),
        }
    }
}
