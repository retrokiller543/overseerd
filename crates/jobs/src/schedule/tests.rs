use std::time::Duration;

use super::{Schedule, ScheduleKind};

fn interval(raw: &str) -> Duration {
    match Schedule::parse(ScheduleKind::Interval, raw).expect("valid interval") {
        Schedule::Every(dur) => dur,
        Schedule::Cron(_) => panic!("interval kind parsed as cron"),
    }
}

#[test]
fn parses_interval_units() {
    assert_eq!(interval("500ms"), Duration::from_millis(500));
    assert_eq!(interval("30s"), Duration::from_secs(30));
    assert_eq!(interval("5m"), Duration::from_secs(300));
    assert_eq!(interval("2h"), Duration::from_secs(7200));
    assert_eq!(interval("1d"), Duration::from_secs(86_400));
}

#[test]
fn parses_compound_interval() {
    assert_eq!(interval("1h 30m"), Duration::from_secs(5400));
}

#[test]
fn parses_month_interval_as_fixed_approximation() {
    // `months` is a fixed 30.44-day approximation, not a calendar month.
    assert_eq!(interval("2 months"), Duration::from_secs(2 * 2_630_016));
}

#[test]
fn trims_interval_whitespace() {
    assert_eq!(interval("  10s "), Duration::from_secs(10));
}

#[test]
fn rejects_bad_intervals() {
    for raw in ["", "s", "10x", "-5s", "every day"] {
        assert!(
            Schedule::parse(ScheduleKind::Interval, raw).is_err(),
            "expected '{raw}' to be rejected"
        );
    }
}

#[test]
fn parses_cron_expressions() {
    assert!(matches!(
        Schedule::parse(ScheduleKind::Cron, "0 3 * * *").expect("valid cron"),
        Schedule::Cron(_)
    ));
}

#[test]
fn parses_cron_nicknames() {
    for nick in [
        "@hourly",
        "@daily",
        "@weekly",
        "@monthly",
        "@yearly",
        "@annually",
    ] {
        assert!(
            matches!(
                Schedule::parse(ScheduleKind::Cron, nick),
                Ok(Schedule::Cron(_))
            ),
            "expected nickname '{nick}' to parse"
        );
    }
}

#[test]
fn rejects_bad_cron() {
    assert!(Schedule::parse(ScheduleKind::Cron, "not a cron").is_err());
    assert!(Schedule::parse(ScheduleKind::Cron, "99 99 99 99 99").is_err());
}
