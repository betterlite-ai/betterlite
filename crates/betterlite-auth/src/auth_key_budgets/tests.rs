#![cfg(test)]

mod email;
mod money;
mod temporary;

use std::collections::BTreeMap;
use std::mem::take;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, FixedOffset, TimeZone as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use tokio::sync::oneshot;
use tokio_util::task::TaskTracker;

use crate::budget_primitives::window_start::DurationParser;
use crate::current_spend::SpendQuery;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::alerts::{BudgetAlert, BudgetAlertKind, BudgetAlertSink, EventGroup};
use super::money::Money;
use super::spend::{SpendReadError, SpendReader};
use super::windows::WindowError;
use super::{Admission, KeyBudgetError, admit, effective_budgets};

fn key(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(fields).unwrap() else {
        panic!("the token fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn emails(text: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(map) = json(text) else {
        panic!("the email config must be an object");
    };
    map
}

fn dec(value: i64) -> Decimal {
    Decimal::from(value)
}

fn dec_text(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn money(value: i64) -> Money {
    Money::Finite(dec(value))
}

fn instant(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap()
}

struct FixedDurations;

impl DurationParser for FixedDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1m" => Some(60_i64),
            "1h" => Some(3_600_i64),
            "2h" => Some(7_200_i64),
            _ => None,
        }
    }
}

fn admission<'request>(
    reader: &'request dyn SpendReader,
    alerts: Option<&'request dyn BudgetAlertSink>,
) -> Admission<'request> {
    Admission {
        reader,
        alerts,
        durations: &FixedDurations,
        global_alert_emails: None,
        alert_fraction: Money::from_f64(0.8_f64),
        throttle_pct: None,
        now: now(),
        user_email: None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedQuery {
    counter_key: String,
    fallback_spend: Decimal,
    max_budget: Option<Decimal>,
    entity_type: Option<String>,
    entity_id: Option<String>,
    duration: Option<String>,
    start: Option<Timestamp>,
    fallback_authoritative: bool,
}

fn snapshot(query: &SpendQuery<'_>) -> RecordedQuery {
    RecordedQuery {
        counter_key: query.counter_key.to_owned(),
        fallback_spend: query.fallback_spend,
        max_budget: query.max_budget,
        entity_type: query
            .window
            .entity_type
            .map(|entity| entity.wire_value().to_owned()),
        entity_id: query.window.entity_id.map(str::to_owned),
        duration: query.window.duration.map(str::to_owned),
        start: query.window.start,
        fallback_authoritative: query.fallback_authoritative,
    }
}

#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<&'static str>>>);

impl EventLog {
    fn push(&self, event: &'static str) {
        self.0.lock().unwrap().push(event);
    }

    fn events(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

struct ScriptedReader {
    spends: Mutex<Vec<Decimal>>,
    queries: Mutex<Vec<RecordedQuery>>,
    log: EventLog,
}

impl ScriptedReader {
    fn new<I>(spends: I) -> Self
    where
        I: IntoIterator<Item = Decimal>,
    {
        Self {
            spends: Mutex::new(spends.into_iter().collect()),
            queries: Mutex::new(Vec::new()),
            log: EventLog::default(),
        }
    }

    fn with_log(self, log: EventLog) -> Self {
        Self { log, ..self }
    }

    fn queries(&self) -> Vec<RecordedQuery> {
        self.queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendReader for ScriptedReader {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        self.queries.lock().unwrap().push(snapshot(query));
        self.log.push("read");
        let mut spends = self.spends.lock().unwrap();
        assert!(!spends.is_empty(), "the reader ran out of scripted spends");
        Ok(spends.remove(0))
    }
}

#[derive(Default)]
struct RecordingSink {
    alerts: Mutex<Vec<BudgetAlert>>,
    log: EventLog,
}

impl RecordingSink {
    fn with_log(self, log: EventLog) -> Self {
        Self { log, ..self }
    }

    fn take(&self) -> Vec<BudgetAlert> {
        take(&mut *self.alerts.lock().unwrap())
    }
}

impl BudgetAlertSink for RecordingSink {
    fn schedule(&self, alert: BudgetAlert) {
        self.log.push("alert");
        self.alerts.lock().unwrap().push(alert);
    }
}

struct DeferredSink {
    tasks: TaskTracker,
    senders: Mutex<Vec<oneshot::Sender<()>>>,
    ran: Arc<AtomicBool>,
}

impl DeferredSink {
    fn new(ran: Arc<AtomicBool>) -> Self {
        Self {
            tasks: TaskTracker::new(),
            senders: Mutex::new(Vec::new()),
            ran,
        }
    }
}

impl BudgetAlertSink for DeferredSink {
    fn schedule(&self, _alert: BudgetAlert) {
        let (sender, receiver) = oneshot::channel::<()>();
        self.senders.lock().unwrap().push(sender);
        let ran = Arc::clone(&self.ran);
        self.tasks.spawn(async move {
            receiver.await.unwrap();
            ran.store(true, Ordering::SeqCst);
        });
    }
}

#[tokio::test]
async fn no_max_no_io() {
    let mut auth = key(r#"{"token":"sk-nomax","spend":3}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new(Vec::new());
    let sink = RecordingSink::default();
    admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap();
    assert!(reader.queries().is_empty());
    assert!(sink.take().is_empty());
}

#[tokio::test]
async fn cross_pod_hard_counter() {
    let mut auth = key(r#"{"token":"sk-hard","spend":1,"max_budget":5}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(3)]);
    let sink = RecordingSink::default();
    admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap();
    let queries = reader.queries();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].counter_key, "spend:key:sk-hard");
    assert_eq!(queries[0].fallback_spend, dec(1));
    assert_eq!(queries[0].max_budget, Some(dec(5)));
    assert_eq!(queries[0].entity_type, None);
    assert_eq!(queries[0].entity_id, None);
    assert_eq!(queries[0].duration, None);
    assert!(queries[0].start.is_none());
    assert!(!queries[0].fallback_authoritative);
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, BudgetAlertKind::TokenBudget);
    assert_eq!(alerts[0].spend, dec(3));
}

#[tokio::test]
async fn alert_before_hard_result() {
    let log = EventLog::default();
    let reader = ScriptedReader::new([dec(6)]).with_log(log.clone());
    let sink = RecordingSink::default().with_log(log.clone());
    let mut auth = key(r#"{"token":"sk-over","spend":0,"max_budget":5}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    let error = admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap_err();
    let KeyBudgetError::Denied(denial) = error else {
        panic!("the request must be denied");
    };
    assert_eq!(denial.current_cost, dec(6));
    assert_eq!(denial.max_budget, dec(5));
    assert_eq!(log.events(), vec!["read", "alert"]);
    assert_eq!(sink.take()[0].spend, dec(6));
}

#[tokio::test]
async fn equal_max_denied() {
    let reader = ScriptedReader::new([dec(5)]);
    let sink = RecordingSink::default();
    let mut auth = key(r#"{"token":"sk-equal","spend":5,"max_budget":5}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    let error = admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap_err();
    let KeyBudgetError::Denied(denial) = error else {
        panic!("spend at the maximum must be denied");
    };
    assert_eq!(denial.entity_type, "key");
    assert_eq!(
        denial
            .entity_id
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("sk-equal".to_owned())
    );
    assert!(!format!("{denial:?}").contains("sk-equal"));
    assert_eq!(denial.status_code, 422_u16);
    assert_eq!(denial.current_cost, dec(5));
    assert_eq!(denial.max_budget, dec(5));
    assert_eq!(
        denial.message,
        "Budget has been exceeded! Key=key Current cost: 5, Max budget: 5"
    );
}

#[rstest]
#[case::nan(Money::Nan)]
#[case::positive_infinity(Money::PositiveInfinity)]
#[case::negative_infinity(Money::NegativeInfinity)]
#[tokio::test]
async fn nonfinite_max_not_denied(#[case] maximum: Money) {
    let mut auth = key(r#"{"token":"sk-nonfinite","spend":0}"#);
    let mut budgets = effective_budgets(&auth, now()).unwrap();
    budgets.max_budget = Some(maximum);
    let reader = ScriptedReader::new([dec(10)]);
    admit(&mut auth, &budgets, &admission(&reader, None))
        .await
        .unwrap();
    assert_eq!(reader.queries()[0].max_budget, None);
}

#[tokio::test]
async fn throttle_request_only() {
    let reader = ScriptedReader::new([dec(6)]);
    let sink = RecordingSink::default();
    let mut auth = key(
        r#"{"token":"sk-throttle","spend":0,"max_budget":5,"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let mut call = admission(&reader, Some(&sink));
    call.throttle_pct = Some(0.5_f64);
    admit(&mut auth, &budgets, &call).await.unwrap();
    assert_eq!(auth.extra.budget_throttle_pct, Some(dec_text("0.5")));
    assert_eq!(auth.view.token.tpm_limit, Some(10_i64));
    assert_eq!(auth.view.token.rpm_limit, None);
    assert_eq!(sink.take().len(), 1);
}

#[tokio::test]
async fn unmasked_name_not_echoed() {
    let reader = ScriptedReader::new([dec(5)]);
    let mut auth = key(
        r#"{"token":"sk-raw","spend":0,"max_budget":5,"key_alias":"prod-key","key_name":"sk-actualsecret"}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let error = admit(&mut auth, &budgets, &admission(&reader, None))
        .await
        .unwrap_err();
    let KeyBudgetError::Denied(denial) = error else {
        panic!("the request must be denied");
    };
    assert_eq!(
        denial.message,
        "Budget has been exceeded! Key=prod-key Current cost: 5, Max budget: 5"
    );
    assert!(!denial.message.contains("sk-actualsecret"));
}

#[rstest]
#[case::four_chars(
    "sk-...abcd",
    "Budget has been exceeded! Key=alias (sk-...abcd) Current cost: 5, Max budget: 5"
)]
#[case::bare_prefix(
    "sk-...",
    "Budget has been exceeded! Key=alias (sk-...) Current cost: 5, Max budget: 5"
)]
#[case::five_chars(
    "sk-...abcde",
    "Budget has been exceeded! Key=alias Current cost: 5, Max budget: 5"
)]
#[case::space_in_suffix(
    "sk-...ab d",
    "Budget has been exceeded! Key=alias Current cost: 5, Max budget: 5"
)]
#[tokio::test]
async fn masked_name_regex(#[case] key_name: &str, #[case] expected: &str) {
    let fields = format!(
        r#"{{"token":"sk-mask","spend":0,"max_budget":5,"key_alias":"alias","key_name":"{key_name}"}}"#
    );
    let mut auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(5)]);
    let error = admit(&mut auth, &budgets, &admission(&reader, None))
        .await
        .unwrap_err();
    let KeyBudgetError::Denied(denial) = error else {
        panic!("the request must be denied");
    };
    assert_eq!(denial.message, expected);
}

#[rstest]
#[case::absent(None, true)]
#[case::null(Some("null"), true)]
#[case::false_flag(Some("false"), true)]
#[case::zero(Some("0"), true)]
#[case::empty_string(Some(r#""""#), true)]
#[case::empty_array(Some("[]"), true)]
#[case::empty_object(Some("{}"), true)]
#[case::mapping(Some(r#"{"80":["owner@example.com"]}"#), true)]
#[case::true_flag(Some("true"), false)]
#[case::nonzero_number(Some("0.5"), false)]
#[case::text(Some(r#""owner@example.com""#), false)]
#[case::nonempty_array(Some(r#"["owner@example.com"]"#), false)]
#[tokio::test]
async fn alert_config_rejects_truthy_non_mappings(
    #[case] alert_config: Option<&str>,
    #[case] accepted: bool,
) {
    let metadata = alert_config.map_or_else(
        || r#"{"token":"sk-alert-config","spend":5,"max_budget":100}"#.to_owned(),
        |config| {
            format!(
                r#"{{"token":"sk-alert-config","spend":5,"max_budget":100,"metadata":{{"max_budget_alert_emails":{config}}}}}"#
            )
        },
    );
    let mut auth = key(&metadata);
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(5)]);
    let result = admit(&mut auth, &budgets, &admission(&reader, None)).await;
    match result {
        Ok(()) => assert!(accepted, "a false value or a mapping must be accepted"),
        Err(KeyBudgetError::Alert(_)) => {
            assert!(!accepted, "a truthy non-mapping must name the field");
        }
        Err(other) => panic!("the config must not reach another check: {other:?}"),
    }
}

#[tokio::test]
async fn each_call_info_field() {
    let reader = ScriptedReader::new([dec(2)]);
    let sink = RecordingSink::default();
    let mut auth = key(
        r#"{"token":"sk-info","spend":1,"max_budget":10,"soft_budget":4,"key_alias":"alias","key_name":"sk-...wxyz","user_id":"user-1","team_id":"team-1","org_id":"org-1","team_alias":"team-alias"}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let mut call = admission(&reader, Some(&sink));
    call.user_email = Some("owner@example.com");
    admit(&mut auth, &budgets, &call).await.unwrap();
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    let alert = &alerts[0];
    assert_eq!(alert.kind, BudgetAlertKind::TokenBudget);
    assert_eq!(alert.spend, dec(2));
    assert_eq!(alert.max_budget, Some(money(10)));
    assert_eq!(alert.soft_budget, Some(money(4)));
    assert_eq!(
        alert
            .token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("sk-info".to_owned())
    );
    assert!(!format!("{alert:?}").contains("sk-info"));
    assert_eq!(alert.user_id.as_deref(), Some("user-1"));
    assert_eq!(alert.team_id.as_deref(), Some("team-1"));
    assert_eq!(alert.team_alias, None);
    assert_eq!(alert.organization_id.as_deref(), Some("org-1"));
    assert_eq!(alert.user_email.as_deref(), Some("owner@example.com"));
    assert_eq!(alert.key_alias.as_deref(), Some("alias"));
    assert_eq!(alert.event_group, EventGroup::Key);
    assert_eq!(alert.max_budget_alert_emails, None);
}

#[rstest]
#[case::minutes(
    0_usize,
    "spend:key:sk-win:window:1m",
    5_i64,
    "1m",
    "2026-01-15T11:59:00+00:00"
)]
#[case::hours(
    1_usize,
    "spend:key:sk-win:window:2h",
    50_i64,
    "2h",
    "2026-01-15T10:00:00+00:00"
)]
#[tokio::test]
async fn window_duration_counter(
    #[case] index: usize,
    #[case] counter_key: &str,
    #[case] limit: i64,
    #[case] duration: &str,
    #[case] start: &str,
) {
    let mut auth = key(
        r#"{"token":"sk-win","spend":0,"budget_limits":[{"budget_duration":"1m","max_budget":5},{"budget_duration":"2h","max_budget":50}]}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(1), dec(2)]);
    admit(&mut auth, &budgets, &admission(&reader, None))
        .await
        .unwrap();
    let queries = reader.queries();
    assert_eq!(queries.len(), 2);
    let query = &queries[index];
    assert_eq!(query.counter_key, counter_key);
    assert_eq!(query.fallback_spend, Decimal::ZERO);
    assert_eq!(query.max_budget, Some(dec(limit)));
    assert_eq!(query.entity_type.as_deref(), Some("key"));
    assert_eq!(query.entity_id.as_deref(), Some("sk-win"));
    assert_eq!(query.duration.as_deref(), Some(duration));
    assert_eq!(query.start, Some(Timestamp::Aware(instant(start))));
    assert!(!query.fallback_authoritative);

    let empty = ScriptedReader::new(Vec::new());
    let mut no_duration = key(r#"{"token":"sk-win","budget_limits":[{"max_budget":5}]}"#);
    let no_duration_budgets = effective_budgets(&no_duration, now()).unwrap();
    let duration_error = admit(
        &mut no_duration,
        &no_duration_budgets,
        &admission(&empty, None),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        duration_error,
        KeyBudgetError::Window(WindowError::Duration { index: 0 })
    ));
    assert!(empty.queries().is_empty());

    let mut no_max = key(r#"{"token":"sk-win","budget_limits":[{"budget_duration":"1m"}]}"#);
    let no_max_budgets = effective_budgets(&no_max, now()).unwrap();
    let max_error = admit(&mut no_max, &no_max_budgets, &admission(&empty, None))
        .await
        .unwrap_err();
    assert!(matches!(
        max_error,
        KeyBudgetError::Window(WindowError::MaxBudget { index: 0 })
    ));
}

#[tokio::test]
async fn window_start_forwarded() {
    let mut auth = key(
        r#"{"token":"sk-start","spend":0,"budget_limits":[{"budget_duration":"1h","max_budget":5,"reset_at":"2026-06-01T00:00:00Z"}]}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(1)]);
    let mut call = admission(&reader, None);
    call.now = instant("2026-06-01T12:00:00Z").with_timezone(&Utc);
    admit(&mut auth, &budgets, &call).await.unwrap();
    assert_eq!(
        reader.queries()[0].start,
        Some(Timestamp::Aware(instant("2026-05-31T23:00:00+00:00")))
    );
}

#[tokio::test]
async fn first_window_stops() {
    let mut auth = key(
        r#"{"token":"sk-first","spend":0,"budget_limits":[{"budget_duration":"1m","max_budget":5},{"budget_duration":"2h","max_budget":50}]}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(7)]);
    let error = admit(&mut auth, &budgets, &admission(&reader, None))
        .await
        .unwrap_err();
    let KeyBudgetError::Denied(denial) = error else {
        panic!("the first window must deny");
    };
    assert_eq!(
        denial.message,
        "ExceededBudget: Key over 1m budget. Spend=$7.0000, Limit=$5.00"
    );
    assert_eq!(denial.entity_type, "key");
    assert_eq!(
        denial
            .entity_id
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("sk-first".to_owned())
    );
    assert_eq!(denial.status_code, 422_u16);
    assert_eq!(reader.queries().len(), 1);
}

#[tokio::test]
async fn multi_window_no_throttle() {
    let mut auth = key(
        r#"{"token":"sk-win-throttle","spend":0,"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10,"budget_limits":[{"budget_duration":"1m","max_budget":5}]}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    let reader = ScriptedReader::new([dec(6)]);
    let mut call = admission(&reader, None);
    call.throttle_pct = Some(0.5_f64);
    let error = admit(&mut auth, &budgets, &call).await.unwrap_err();
    assert!(matches!(error, KeyBudgetError::Denied(_)));
    assert_eq!(auth.extra.budget_throttle_pct, None);
}

#[rstest]
#[case::zero_spend(0_i64)]
#[case::at_soft_budget(5_i64)]
#[tokio::test]
async fn zero_soft_budget_skips(#[case] spend: i64) {
    let sink = RecordingSink::default();
    let reader = ScriptedReader::new(Vec::new());
    let fields = format!(r#"{{"token":"sk-zero-soft","spend":{spend},"soft_budget":0}}"#);
    let mut auth = key(&fields);
    let budgets = effective_budgets(&auth, now()).unwrap();
    admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap();
    assert!(sink.take().is_empty());
}

#[tokio::test]
async fn soft_uses_token_spend() {
    let sink = RecordingSink::default();
    let reader = ScriptedReader::new([dec(1)]);
    let mut auth = key(r#"{"token":"sk-soft","spend":5,"max_budget":10,"soft_budget":4}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap();
    let alerts = sink.take();
    assert_eq!(alerts.len(), 2);
    assert_eq!(alerts[0].kind, BudgetAlertKind::TokenBudget);
    assert_eq!(alerts[0].spend, dec(1));
    assert_eq!(alerts[1].kind, BudgetAlertKind::SoftBudget);
    assert_eq!(alerts[1].spend, dec(5));
    assert_eq!(reader.queries().len(), 1);
}

#[tokio::test]
async fn equal_soft_alert_only() {
    let sink = RecordingSink::default();
    let reader = ScriptedReader::new(Vec::new());
    let mut auth =
        key(r#"{"token":"sk-soft-equal","spend":5,"soft_budget":5,"team_alias":"team-a"}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap();
    let alerts = sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, BudgetAlertKind::SoftBudget);
    assert_eq!(alerts[0].spend, dec(5));
    assert_eq!(alerts[0].soft_budget, Some(money(5)));
    assert_eq!(alerts[0].max_budget, None);
    assert_eq!(alerts[0].team_alias.as_deref(), Some("team-a"));
    assert_eq!(alerts[0].event_group, EventGroup::Key);
}

#[tokio::test]
async fn alerts_not_awaited() {
    let ran = Arc::new(AtomicBool::new(false));
    let sink = DeferredSink::new(Arc::clone(&ran));
    let reader = ScriptedReader::new(Vec::new());
    let mut auth = key(r#"{"token":"sk-deferred","spend":5,"soft_budget":5}"#);
    let budgets = effective_budgets(&auth, now()).unwrap();
    admit(&mut auth, &budgets, &admission(&reader, Some(&sink)))
        .await
        .unwrap();
    assert_eq!(sink.tasks.len(), 1);
    assert!(!ran.load(Ordering::SeqCst));
    let sender = sink.senders.lock().unwrap().pop().unwrap();
    sender.send(()).unwrap();
    sink.tasks.close();
    sink.tasks.wait().await;
    assert!(ran.load(Ordering::SeqCst));
}
