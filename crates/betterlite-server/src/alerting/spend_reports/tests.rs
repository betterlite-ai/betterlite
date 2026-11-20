#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::analytics::python::float_text;
use chrono::{DateTime, NaiveDate, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{
    SpendAlertCache, SpendAlertClock, SpendAlertSender, SpendReports, float_rounded, parse_days,
};
use crate::spend::estimates::report::{ReportError, SpendReportSource, SpendRow, SpendTotals};

const LOCAL_TODAY: &str = "2026-01-15";

#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheWrite {
    key: String,
    value: String,
    ttl_seconds: u64,
}

#[derive(Clone, Default)]
struct FakeCache {
    entries: Arc<Mutex<BTreeMap<String, String>>>,
    writes: Arc<Mutex<Vec<CacheWrite>>>,
}

impl FakeCache {
    fn seeded(key: &str) -> Self {
        let cache = Self::default();
        cache
            .entries
            .lock()
            .unwrap()
            .insert(key.to_owned(), "SENT".to_owned());
        cache
    }

    fn written(&self) -> Vec<CacheWrite> {
        self.writes.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendAlertCache for FakeCache {
    async fn get(&self, key: &str) -> Option<String> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        self.writes.lock().unwrap().push(CacheWrite {
            key: key.to_owned(),
            value: value.to_owned(),
            ttl_seconds,
        });
    }
}

#[derive(Clone, Debug)]
struct SentAlert {
    message: String,
    level: AlertLevel,
    alert_type: AlertType,
    metadata_entries: usize,
    kwargs_entries: usize,
}

impl SentAlert {
    fn of(alert: Alert<'_>) -> Self {
        Self {
            message: alert.message.to_owned(),
            level: alert.level,
            alert_type: alert.alert_type,
            metadata_entries: alert.alerting_metadata.len(),
            kwargs_entries: alert.kwargs.len(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake sender failed")]
struct FakeSendError;

#[derive(Clone, Default)]
struct FakeSender {
    sent: Arc<Mutex<Vec<SentAlert>>>,
    fails: bool,
}

impl FakeSender {
    fn failing() -> Self {
        Self {
            sent: Arc::default(),
            fails: true,
        }
    }

    fn alerts(&self) -> Vec<SentAlert> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendAlertSender for FakeSender {
    type Error = FakeSendError;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeSendError);
        }
        self.sent.lock().unwrap().push(SentAlert::of(alert));
        Ok(())
    }
}

type Asked = Arc<Mutex<Vec<(SpendTotals, NaiveDate, NaiveDate)>>>;

#[derive(Clone, Default)]
struct FakeSource {
    teams: Vec<SpendRow>,
    tags: Vec<SpendRow>,
    asked: Asked,
}

impl FakeSource {
    fn answering(teams: Vec<SpendRow>, tags: Vec<SpendRow>) -> Self {
        Self {
            teams,
            tags,
            asked: Arc::default(),
        }
    }

    fn queries(&self) -> Asked {
        self.asked.clone()
    }
}

#[async_trait]
impl SpendReportSource for FakeSource {
    async fn totals(
        &self,
        totals: SpendTotals,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<SpendRow>, ReportError> {
        self.asked
            .lock()
            .unwrap()
            .push((totals, start.date_naive(), end.date_naive()));
        Ok(match totals {
            SpendTotals::Teams => self.teams.clone(),
            SpendTotals::Tags => self.tags.clone(),
        })
    }
}

#[derive(Clone, Copy)]
struct FakeClock {
    local: NaiveDate,
    utc: NaiveDate,
}

impl FakeClock {
    fn on(local: &str) -> Self {
        Self {
            local: date(local),
            utc: date("1999-12-31"),
        }
    }
}

impl SpendAlertClock for FakeClock {
    fn local_date(&self) -> NaiveDate {
        self.local
    }

    fn utc_date(&self) -> NaiveDate {
        self.utc
    }
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::default());
    let writer = captured.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Captured(Arc::clone(&writer)))
        .with_max_level(tracing::Level::ERROR)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

fn row(name: Option<&str>, spend: &str) -> SpendRow {
    SpendRow {
        name: name.map(str::to_owned),
        total_spend: decimal(spend),
    }
}

fn service(
    cache: FakeCache,
    sender: FakeSender,
    source: FakeSource,
    clock: FakeClock,
) -> SpendReports<FakeCache, FakeSender, FakeClock> {
    SpendReports {
        alerting: Some(vec![AlertDestination::Slack]),
        alert_types: vec![AlertType::SpendReports],
        alerting_args: SlackAlertingArgs::default(),
        hours_in_a_day: 24,
        cache,
        sender,
        source: Some(Box::new(source)),
        clock,
    }
}

fn reporting(
    cache: FakeCache,
    sender: FakeSender,
    source: FakeSource,
) -> SpendReports<FakeCache, FakeSender, FakeClock> {
    service(cache, sender, source, FakeClock::on(LOCAL_TODAY))
}

fn report_rows() -> (Vec<SpendRow>, Vec<SpendRow>) {
    (
        vec![
            row(Some("team-a"), "12.34567"),
            row(None, "1"),
            row(Some("team-c"), "0.00005"),
            row(Some("team-d"), "0.00001"),
        ],
        vec![row(Some("tag-a"), "3.5")],
    )
}

fn asked_dates(asked: &Asked) -> Vec<(String, String)> {
    asked
        .lock()
        .unwrap()
        .iter()
        .map(|(_, start, end)| {
            (
                start.format("%Y-%m-%d").to_string(),
                end.format("%Y-%m-%d").to_string(),
            )
        })
        .collect()
}

const WEEKLY_MESSAGE: &str = "*💸 Spend Report for `01-08-2026 - 01-15-2026` (7 days)*\n\n*Team Spend Report:*\nTeam: `team-a` | Spend: `$12.3457`\nTeam: `None` | Spend: `$1.0`\nTeam: `team-c` | Spend: `$0.0001`\nTeam: `team-d` | Spend: `$0.0`\n\n*Tag Spend Report:*\nTag: `tag-a` | Spend: `$3.5`\n";

const MONTHLY_MESSAGE: &str = "*💸 Monthly Spend Report for `01-01-2026 - 01-31-2026` *\n\n*Team Spend Report:*\nTeam: `team-a` | Spend: `$12.3457`\nTeam: `None` | Spend: `$1.0`\nTeam: `team-c` | Spend: `$0.0001`\nTeam: `team-d` | Spend: `$0.0`\n\n*Tag Spend Report:*\nTag: `tag-a` | Spend: `$3.5`\n";

fn answering(teams: Vec<SpendRow>, tags: Vec<SpendRow>) -> FakeSource {
    FakeSource::answering(teams, tags)
}

#[tokio::test]
async fn weekly_report_returns_without_alerting() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let mut reports = reporting(FakeCache::default(), FakeSender::default(), source);
    reports.alerting = None;
    reports.send_weekly_spend_report(None).await;
    assert!(asked.lock().unwrap().is_empty());
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[tokio::test]
async fn weekly_report_returns_without_the_spend_reports_type() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let mut reports = reporting(FakeCache::default(), FakeSender::default(), source);
    reports.alert_types = vec![AlertType::BudgetAlerts];
    reports.send_weekly_spend_report(None).await;
    assert!(asked.lock().unwrap().is_empty());
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[tokio::test]
async fn weekly_report_returns_when_the_marker_is_set() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let reports = reporting(
        FakeCache::seeded("weekly_spend_report_sent_2026-01-08_2026-01-15"),
        FakeSender::default(),
        source,
    );
    reports.send_weekly_spend_report(None).await;
    assert!(asked.lock().unwrap().is_empty());
    assert!(reports.sender.alerts().is_empty());
}

#[tokio::test]
async fn weekly_report_returns_when_there_is_no_source() {
    let mut reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.source = None;
    reports.send_weekly_spend_report(None).await;
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[tokio::test]
async fn weekly_report_returns_when_both_lists_are_empty() {
    let reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(Vec::new(), Vec::new()),
    );
    reports.send_weekly_spend_report(None).await;
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[tokio::test]
async fn weekly_report_renders_python_spend_text_and_a_none_alias() {
    let (teams, tags) = report_rows();
    let reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(teams, tags),
    );
    reports.send_weekly_spend_report(None).await;
    let sent = reports.sender.alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, WEEKLY_MESSAGE);
}

#[tokio::test]
async fn weekly_report_sends_low_with_empty_metadata_then_writes_the_marker() {
    let (teams, tags) = report_rows();
    let reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(teams, tags),
    );
    reports.send_weekly_spend_report(None).await;
    let sent = reports.sender.alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Low);
    assert_eq!(sent[0].alert_type, AlertType::SpendReports);
    assert_eq!(sent[0].metadata_entries, 0);
    assert_eq!(sent[0].kwargs_entries, 0);
    assert_eq!(
        reports.cache.written(),
        vec![CacheWrite {
            key: "weekly_spend_report_sent_2026-01-08_2026-01-15".to_owned(),
            value: "SENT".to_owned(),
            ttl_seconds: 604_800,
        }]
    );
}

#[tokio::test]
async fn weekly_report_queries_teams_then_tags_for_the_window() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let reports = reporting(FakeCache::default(), FakeSender::default(), source);
    reports.send_weekly_spend_report(None).await;
    let asked_kinds: Vec<SpendTotals> = asked
        .lock()
        .unwrap()
        .iter()
        .map(|(kind, ..)| *kind)
        .collect();
    assert_eq!(asked_kinds, vec![SpendTotals::Teams, SpendTotals::Tags]);
    assert_eq!(
        asked_dates(&asked),
        vec![
            ("2026-01-08".to_owned(), "2026-01-15".to_owned()),
            ("2026-01-08".to_owned(), "2026-01-15".to_owned()),
        ]
    );
}

#[tokio::test]
async fn weekly_report_defaults_to_seven_days() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let reports = reporting(FakeCache::default(), FakeSender::default(), source);
    reports.send_weekly_spend_report(None).await;
    assert_eq!(
        asked_dates(&asked),
        vec![("2026-01-08".to_owned(), "2026-01-15".to_owned())]
    );
    assert_eq!(reports.cache.written()[0].ttl_seconds, 604_800);
}

#[tokio::test]
async fn weekly_report_uses_the_local_date_for_the_window() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let clock = FakeClock {
        local: date("2026-03-01"),
        utc: date("2026-02-28"),
    };
    let reports = service(FakeCache::default(), FakeSender::default(), source, clock);
    reports.send_weekly_spend_report(Some("1d")).await;
    assert_eq!(
        asked_dates(&asked),
        vec![("2026-02-28".to_owned(), "2026-03-01".to_owned())]
    );
    assert_eq!(
        reports.cache.written()[0].key,
        "weekly_spend_report_sent_2026-02-28_2026-03-01"
    );
}

#[rstest]
#[case::missing_unit("7")]
#[case::wrong_unit("7m")]
#[case::empty("")]
#[case::negative("-1d")]
#[case::unit_only("d")]
#[tokio::test]
async fn weekly_report_logs_an_invalid_time_range_and_sends_nothing(#[case] time_range: &str) {
    let (logs, _guard) = capture_logs();
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let reports = reporting(FakeCache::default(), FakeSender::default(), source);
    reports.send_weekly_spend_report(Some(time_range)).await;
    assert!(logs.text().contains("Invalid time range format"));
    assert!(asked.lock().unwrap().is_empty());
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[rstest]
#[case::lowercase("7d", 7)]
#[case::uppercase("7D", 7)]
#[case::plus_sign("+7d", 7)]
#[case::zero("0d", 0)]
#[case::padded("007d", 7)]
fn weekly_time_range_parses_the_day_count(#[case] text: &str, #[case] days: u32) {
    assert_eq!(parse_days(text).unwrap(), days);
}

#[rstest]
#[case::sign("-7d")]
#[case::spaces(" 7d")]
#[case::underscore("7_0d")]
#[case::letters("dd")]
fn weekly_time_range_rejects_a_non_day_count(#[case] text: &str) {
    parse_days(text).unwrap_err();
}

#[tokio::test]
async fn weekly_report_accepts_an_uppercase_unit() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let reports = reporting(FakeCache::default(), FakeSender::default(), source);
    reports.send_weekly_spend_report(Some("7D")).await;
    assert_eq!(
        asked_dates(&asked),
        vec![("2026-01-08".to_owned(), "2026-01-15".to_owned())]
    );
    assert_eq!(reports.cache.written()[0].ttl_seconds, 604_800);
}

#[tokio::test]
async fn weekly_report_writes_no_marker_when_the_sender_fails() {
    let (logs, _guard) = capture_logs();
    let reports = reporting(
        FakeCache::default(),
        FakeSender::failing(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.send_weekly_spend_report(None).await;
    assert!(logs.text().contains("Error sending spend report"));
    assert!(reports.cache.written().is_empty());
}

#[rstest]
#[case::rounded_up("12.34567", "12.3457")]
#[case::binary_above_the_tie("0.00005", "0.0001")]
#[case::binary_below_the_tie("0.00001", "0.0")]
#[case::whole("1", "1.0")]
#[case::zero("0", "0.0")]
#[case::negative("-0.00005", "-0.0001")]
fn weekly_report_rounds_spend_like_python(#[case] amount: &str, #[case] shown: &str) {
    assert_eq!(
        float_text(float_rounded(decimal(amount), 4).as_f64()),
        shown
    );
}

#[tokio::test]
async fn monthly_report_runs_without_the_spend_reports_type() {
    let mut reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.alert_types = Vec::new();
    reports.send_monthly_spend_report().await;
    assert_eq!(reports.sender.alerts().len(), 1);
    assert_eq!(
        reports.cache.written(),
        vec![CacheWrite {
            key: "monthly_spend_report_sent_2026-01-01_2026-01-31".to_owned(),
            value: "SENT".to_owned(),
            ttl_seconds: 2_592_000,
        }]
    );
}

#[tokio::test]
async fn monthly_report_renders_the_header_with_a_space_before_the_star() {
    let (teams, tags) = report_rows();
    let reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(teams, tags),
    );
    reports.send_monthly_spend_report().await;
    let sent = reports.sender.alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, MONTHLY_MESSAGE);
    assert_eq!(sent[0].level, AlertLevel::Low);
    assert_eq!(sent[0].alert_type, AlertType::SpendReports);
}

#[tokio::test]
async fn monthly_report_prints_the_tag_heading_for_an_empty_tag_list() {
    let reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.send_monthly_spend_report().await;
    assert!(
        reports.sender.alerts()[0]
            .message
            .ends_with("\n\n*Tag Spend Report:*\n")
    );
}

#[tokio::test]
async fn monthly_report_returns_when_the_marker_is_set() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let reports = reporting(
        FakeCache::seeded("monthly_spend_report_sent_2026-01-01_2026-01-31"),
        FakeSender::default(),
        source,
    );
    reports.send_monthly_spend_report().await;
    assert!(asked.lock().unwrap().is_empty());
    assert!(reports.sender.alerts().is_empty());
}

#[tokio::test]
async fn monthly_report_returns_when_there_is_no_source() {
    let mut reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.source = None;
    reports.send_monthly_spend_report().await;
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[tokio::test]
async fn monthly_report_returns_when_both_lists_are_empty() {
    let reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(Vec::new(), Vec::new()),
    );
    reports.send_monthly_spend_report().await;
    assert!(reports.sender.alerts().is_empty());
    assert!(reports.cache.written().is_empty());
}

#[tokio::test]
async fn monthly_report_uses_the_month_window_of_the_local_date() {
    let source = answering(vec![row(Some("team-a"), "2")], Vec::new());
    let asked = source.queries();
    let clock = FakeClock {
        local: date("2026-02-10"),
        utc: date("2026-01-31"),
    };
    let reports = service(FakeCache::default(), FakeSender::default(), source, clock);
    reports.send_monthly_spend_report().await;
    assert_eq!(
        asked_dates(&asked),
        vec![("2026-02-01".to_owned(), "2026-02-28".to_owned())]
    );
    assert_eq!(
        reports.cache.written()[0].key,
        "monthly_spend_report_sent_2026-02-01_2026-02-28"
    );
}

#[tokio::test]
async fn monthly_report_uses_hours_in_a_day_for_the_marker_ttl() {
    let mut reports = reporting(
        FakeCache::default(),
        FakeSender::default(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.hours_in_a_day = 1;
    reports.send_monthly_spend_report().await;
    assert_eq!(reports.cache.written()[0].ttl_seconds, 108_000);
}

#[tokio::test]
async fn monthly_report_logs_a_sender_failure_and_writes_no_marker() {
    let (logs, _guard) = capture_logs();
    let reports = reporting(
        FakeCache::default(),
        FakeSender::failing(),
        answering(vec![row(Some("team-a"), "2")], Vec::new()),
    );
    reports.send_monthly_spend_report().await;
    assert!(logs.text().contains("Error sending monthly spend report"));
    assert!(reports.cache.written().is_empty());
}
