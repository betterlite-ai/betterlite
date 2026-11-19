#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::future::ready;
use std::sync::Arc;

use betterlite_auth::auth_cache::WriteOptions;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use super::data::ManagementData;
use super::eviction::delete_cached_keys;
use super::fixtures::auth_cache::memory_auth_cache;
use super::fixtures::cache::FakeCacheError;
use super::fixtures::clock::{end_instant, start_instant};
use super::fixtures::harness::{
    CreatedKey, Harness, actor_auth, call, created_key, exception, http_request, parent_span,
};
use super::fixtures::journal::{Effect, Journal};
use super::fixtures::transports::SpanHook;
use super::wrapper::{ManagementCall, ManagementFailure, management_endpoint_wrapper};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
enum FakeHandlerError {
    #[error("the handler failed")]
    Failed,
    #[error("the handler rejected {detail}")]
    Rejected { detail: String },
}

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| String::from(*name)).collect()
}

fn expected_response() -> Json {
    Json::Object(IndexMap::from([
        ("alias".to_owned(), Json::Text("ops".to_owned())),
        ("user_id".to_owned(), Json::Text("u1".to_owned())),
    ]))
}

async fn run<Handler>(
    harness: &Harness,
    request: ManagementCall<'_>,
    handler: Handler,
) -> Result<CreatedKey, FakeHandlerError>
where
    Handler: Future<Output = Result<CreatedKey, ManagementFailure<FakeHandlerError>>> + Send,
{
    management_endpoint_wrapper(request, harness.deps(), handler).await
}

fn capture_logs() -> (tempfile::NamedTempFile, DefaultGuard) {
    let log = tempfile::NamedTempFile::new().expect("the log file opens");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("the log file reopens")))
        .finish();
    (log, set_default(subscriber))
}

#[tokio::test]
async fn wall_clock_brackets_handler() {
    let harness = Harness::default();
    let auth = actor_auth();
    let parent = parent_span();
    let probe = Journal::default();
    let seen = probe.clone();
    let clock = Arc::clone(&harness.clock);
    let handler = async move {
        seen.record(clock.samples.entries().len());
        Ok(created_key())
    };
    let outcome = run(&harness, call(None, Some(&auth), Some(&parent)), handler).await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        created_key()
    );
    assert_eq!(probe.entries(), vec![1]);
    let spans = harness.span.sent.entries();
    assert_eq!(spans[0].start_time, SpanTime::DateTime(start_instant()));
    assert_eq!(spans[0].end_time, SpanTime::DateTime(end_instant()));
}

#[tokio::test]
async fn default_auth_when_missing() {
    let harness = Harness::default();
    let parent = parent_span();
    let data = ManagementData::UpdateKey("k");
    let mut request = call(Some(&data), None, None);
    request.parent_otel_span = Some(&parent);
    let outcome = run(&harness, request, ready(Ok(created_key()))).await;
    outcome.unwrap();
    let alerts = harness.alert.sent.entries();
    assert_eq!(alerts[0].created_by_user_id, "Unknown");
    assert_eq!(alerts[0].created_by_user_role, "Unknown");
    assert_eq!(alerts[0].created_by_key_alias, None);
    // a default auth carries no parent span, so the provided one is never used
    assert_eq!(
        harness.events.entries(),
        vec![Effect::Alert, Effect::Delete("k".to_owned())]
    );
}

#[tokio::test]
async fn success_alert_span_eviction_order() {
    let harness = Harness::default();
    let auth = actor_auth();
    let parent = parent_span();
    let data = ManagementData::UpdateKey("k");
    let mut request = call(Some(&data), Some(&auth), Some(&parent));
    request.http_request = Some(http_request("/key/generate"));
    let outcome = run(&harness, request, ready(Ok(created_key()))).await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        created_key()
    );
    assert_eq!(
        harness.events.entries(),
        vec![
            Effect::Alert,
            Effect::SuccessSpan,
            Effect::Delete("k".to_owned()),
        ]
    );
    let alerts = harness.alert.sent.entries();
    assert_eq!(alerts[0].created_by_user_id, "u1");
    assert_eq!(alerts[0].created_by_user_role, "proxy_admin");
    assert_eq!(alerts[0].created_by_key_alias.as_deref(), Some("ops"));
    assert_eq!(alerts[0].alert_type, AlertType::NewVirtualKeyCreated);
    let spans = harness.span.sent.entries();
    assert_eq!(spans[0].route, "/key/generate");
    assert_eq!(spans[0].hook, SpanHook::Success);
    assert_eq!(spans[0].response, Some(expected_response()));
    assert_eq!(spans[0].parent, Some(parent.context.clone()));
}

#[tokio::test]
async fn unprojectable_result_has_no_response() {
    let harness = Harness::default();
    let auth = actor_auth();
    let parent = parent_span();
    // a tuple map key makes serde_json reject the projection, as dict(result) raises in LiteLLM
    let result = BTreeMap::from([((1_u8, 2_u8), 3_u8)]);
    let outcome: Result<BTreeMap<(u8, u8), u8>, FakeHandlerError> = management_endpoint_wrapper(
        call(None, Some(&auth), Some(&parent)),
        harness.deps(),
        ready(Ok(result)),
    )
    .await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        BTreeMap::from([((1_u8, 2_u8), 3_u8)])
    );
    let spans = harness.span.sent.entries();
    assert_eq!(spans[0].response, None);
    assert_eq!(
        harness.events.entries(),
        vec![Effect::Alert, Effect::SuccessSpan]
    );
}

#[tokio::test]
async fn absent_parent_skips_span() {
    let harness = Harness::default();
    let auth = actor_auth();
    let data = ManagementData::UpdateKey("k");
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    assert_eq!(
        harness.events.entries(),
        vec![Effect::Alert, Effect::Delete("k".to_owned())]
    );
    assert!(harness.span.sent.entries().is_empty());
}

#[tokio::test]
async fn alert_failure_stops_later_effects() {
    let mut harness = Harness::default();
    harness.alert.fails = true;
    let ids = keys(&["k1", "k2"]);
    let data = ManagementData::DeleteKeys(&ids);
    let auth = actor_auth();
    let parent = parent_span();
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), Some(&parent)),
        ready(Ok(created_key())),
    )
    .await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        created_key()
    );
    assert!(harness.events.entries().is_empty());
    assert!(harness.span.sent.entries().is_empty());
    assert!(harness.cache.deletes.entries().is_empty());
}

#[tokio::test]
async fn success_span_error_skips_cache() {
    let mut harness = Harness::default();
    harness.span.fails = true;
    let ids = keys(&["k1"]);
    let data = ManagementData::DeleteKeys(&ids);
    let auth = actor_auth();
    let parent = parent_span();
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), Some(&parent)),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    assert_eq!(harness.events.entries(), vec![Effect::Alert]);
    assert!(harness.cache.deletes.entries().is_empty());
}

#[tokio::test]
async fn cache_error_stops_remaining_deletes() {
    let mut harness = Harness::default();
    harness.cache.failing = Some("k2".to_owned());
    let ids = keys(&["k1", "k2", "k3"]);
    let data = ManagementData::DeleteKeys(&ids);
    let auth = actor_auth();
    let parent = parent_span();
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), Some(&parent)),
        ready(Ok(created_key())),
    )
    .await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        created_key()
    );
    assert_eq!(harness.cache.deletes.entries(), keys(&["k1", "k2"]));
    assert_eq!(
        harness.events.entries(),
        vec![
            Effect::Alert,
            Effect::SuccessSpan,
            Effect::Delete("k1".to_owned()),
            Effect::Delete("k2".to_owned()),
        ]
    );
}

#[tokio::test]
async fn debug_not_warning() {
    let (log, _guard) = capture_logs();
    let mut harness = Harness::default();
    harness.cache.failing = Some("k".to_owned());
    let data = ManagementData::UpdateKey("k");
    let outcome = run(
        &harness,
        call(Some(&data), None, None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.contains("DEBUG"));
    assert!(
        written
            .contains("Error in management endpoint wrapper: the fake cache refused to delete k")
    );
    assert!(!written.contains("WARN"));
    assert!(!written.contains("ERROR"));
}

#[tokio::test]
async fn unchanged_result_after_effect_failure() {
    let mut harness = Harness::default();
    harness.span.fails = true;
    let ids = keys(&["u1"]);
    let data = ManagementData::DeleteUsers(&ids);
    let auth = actor_auth();
    let parent = parent_span();
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), Some(&parent)),
        ready(Ok(created_key())),
    )
    .await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        created_key()
    );
    assert!(harness.cache.deletes.entries().is_empty());
}

#[tokio::test]
async fn handler_error_no_alert_or_eviction() {
    let harness = Harness::default();
    let auth = actor_auth();
    let parent = parent_span();
    let ids = keys(&["k"]);
    let data = ManagementData::DeleteKeys(&ids);
    let failure = ManagementFailure {
        exception: exception(),
        error: FakeHandlerError::Failed,
    };
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), Some(&parent)),
        ready(Err(failure)),
    )
    .await;
    outcome.unwrap_err();
    assert!(harness.alert.sent.entries().is_empty());
    assert!(harness.cache.deletes.entries().is_empty());
    assert_eq!(harness.events.entries(), vec![Effect::FailureSpan]);
}

#[tokio::test]
async fn failure_span_keeps_original_exception() {
    let harness = Harness::default();
    let auth = actor_auth();
    let parent = parent_span();
    let failure = ManagementFailure {
        exception: exception(),
        error: FakeHandlerError::Rejected {
            detail: "nope".to_owned(),
        },
    };
    let outcome = run(
        &harness,
        call(None, Some(&auth), Some(&parent)),
        ready(Err(failure)),
    )
    .await;
    assert_eq!(
        outcome.expect_err("the handler error comes back"),
        FakeHandlerError::Rejected {
            detail: "nope".to_owned()
        }
    );
    let spans = harness.span.sent.entries();
    let sent = &spans[0];
    assert_eq!(sent.hook, SpanHook::Failure);
    assert_eq!(sent.response, None);
    let facts = sent
        .exception
        .as_ref()
        .expect("the failure span carries the exception");
    assert_eq!(facts.class_name, "ProxyException");
    assert_eq!(facts.message.as_deref(), Some("boom"));
    assert_eq!(sent.parent, Some(parent.context.clone()));
}

#[tokio::test]
async fn kwargs_data_only() {
    let harness = Harness::default();
    let auth = actor_auth();
    let kwargs = JsonObject::from([(
        "data".to_owned(),
        Json::List(vec![Json::Text("k".to_owned())]),
    )]);
    let mut request = call(None, Some(&auth), None);
    request.request_kwargs = kwargs.clone();
    let outcome = run(&harness, request, ready(Ok(created_key()))).await;
    outcome.unwrap();
    assert!(harness.cache.deletes.entries().is_empty());
    assert_eq!(harness.alert.sent.entries()[0].request_kwargs, kwargs);
    let data = ManagementData::UpdateKey("k");
    let second = run(
        &harness,
        call(Some(&data), None, None),
        ready(Ok(created_key())),
    )
    .await;
    second.unwrap();
    assert_eq!(harness.cache.deletes.entries(), keys(&["k"]));
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    UpdateKey,
    DeleteKeys,
    UpdateUser,
    DeleteUsers,
    UpdateTeam,
    DeleteTeams,
    UpdateCustomer,
    DeleteCustomers,
}

impl Kind {
    fn data(self, ids: &[String]) -> ManagementData<'_> {
        let first = ids.first().expect("an update id").as_str();
        match self {
            Self::UpdateKey => ManagementData::UpdateKey(first),
            Self::DeleteKeys => ManagementData::DeleteKeys(ids),
            Self::UpdateUser => ManagementData::UpdateUser(first),
            Self::DeleteUsers => ManagementData::DeleteUsers(ids),
            Self::UpdateTeam => ManagementData::UpdateTeam(first),
            Self::DeleteTeams => ManagementData::DeleteTeams(ids),
            Self::UpdateCustomer => ManagementData::UpdateCustomer(first),
            Self::DeleteCustomers => ManagementData::DeleteCustomers(ids),
        }
    }
}

#[rstest]
#[case::update_key(Kind::UpdateKey, &["k1"], &["k1"])]
#[case::delete_keys(Kind::DeleteKeys, &["k1", "k2"], &["k1", "k2"])]
#[case::delete_keys_empty_entry(Kind::DeleteKeys, &[""], &[""])]
#[case::update_user(Kind::UpdateUser, &["u1"], &["u1"])]
#[case::delete_users(Kind::DeleteUsers, &["u1", "u2"], &["u1", "u2"])]
#[case::update_team(Kind::UpdateTeam, &["t1"], &["t1"])]
#[case::delete_teams(Kind::DeleteTeams, &["t1", "t2"], &["t1", "t2"])]
#[case::update_customer(Kind::UpdateCustomer, &["c1"], &["c1"])]
#[case::delete_customers(Kind::DeleteCustomers, &["c1", "c2"], &["c1", "c2"])]
#[tokio::test]
async fn typed_request_dispatch(
    #[case] kind: Kind,
    #[case] supplied: &[&str],
    #[case] expected: &[&str],
) {
    let harness = Harness::default();
    let auth = actor_auth();
    let ids = keys(supplied);
    let data = kind.data(&ids);
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    assert_eq!(harness.cache.deletes.entries(), keys(expected));
}

#[tokio::test]
async fn raw_keys_no_prefix_or_hash() {
    let harness = Harness::default();
    let auth = actor_auth();
    let teams = keys(&["team-abc"]);
    let team_data = ManagementData::DeleteTeams(&teams);
    let outcome = run(
        &harness,
        call(Some(&team_data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    let hashed = "a".repeat(64);
    let users = vec![hashed.clone()];
    let user_data = ManagementData::DeleteUsers(&users);
    let second = run(
        &harness,
        call(Some(&user_data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    second.unwrap();
    assert_eq!(
        harness.cache.deletes.entries(),
        vec!["team-abc".to_owned(), hashed]
    );
}

#[tokio::test]
async fn duplicates_keep_order() {
    let harness = Harness::default();
    let auth = actor_auth();
    let ids = keys(&["k2", "k1", "k2", "k1"]);
    let data = ManagementData::DeleteKeys(&ids);
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    assert_eq!(harness.cache.deletes.entries(), ids);
}

#[tokio::test]
async fn null_data_and_other_types_noop() {
    let harness = Harness::default();
    let auth = actor_auth();
    let empty = keys(&[]);
    let empty_data = ManagementData::DeleteKeys(&empty);
    let outcome = run(
        &harness,
        call(Some(&empty_data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    let second = run(
        &harness,
        call(None, Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    second.unwrap();
    assert!(harness.cache.deletes.entries().is_empty());
    let key_data = ManagementData::UpdateKey("k1");
    delete_cached_keys(&harness.cache, Some(&key_data)).expect("the key class deletes its one id");
    let deleted = keys(&["c1"]);
    let customer = ManagementData::DeleteCustomers(&deleted);
    delete_cached_keys(&harness.cache, Some(&customer))
        .expect("the customer class deletes its ids");
    assert_eq!(harness.cache.deletes.entries(), keys(&["k1", "c1"]));
}

#[test]
fn direct_eviction_error_propagates() {
    let mut harness = Harness::default();
    harness.cache.failing = Some("t1".to_owned());
    let data = ManagementData::UpdateTeam("t1");
    let error = delete_cached_keys(&harness.cache, Some(&data))
        .expect_err("the cache error reaches the caller");
    assert_eq!(
        error,
        FakeCacheError::Refused {
            key: "t1".to_owned()
        }
    );
}

#[test]
fn auth_cache_tier_delete_evicts() {
    let cache = memory_auth_cache();
    let value = Json::Text("stored".to_owned());
    cache
        .set(
            Some("sk-live"),
            &value,
            WriteOptions {
                ttl: None,
                local_only: true,
                refresh_ttl: false,
            },
        )
        .expect("the memory tier stores the value");
    assert!(cache.get("sk-live", true).is_some());
    delete_cached_keys(&cache, Some(&ManagementData::UpdateKey("sk-live")))
        .expect("the delete reaches the tier");
    assert!(cache.get("sk-live", true).is_none());
}

#[tokio::test]
async fn success_evicts() {
    let harness = Harness::default();
    let auth = actor_auth();
    let data = ManagementData::UpdateUser("u1");
    let outcome = run(
        &harness,
        call(Some(&data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    assert_eq!(
        outcome.expect("the handler result comes back"),
        created_key()
    );
    assert_eq!(
        harness.events.entries(),
        vec![Effect::Alert, Effect::Delete("u1".to_owned())]
    );
}

#[tokio::test]
async fn handler_failure_preserved() {
    let harness = Harness::default();
    let failure = ManagementFailure {
        exception: exception(),
        error: FakeHandlerError::Rejected {
            detail: "denied".to_owned(),
        },
    };
    let outcome = run(&harness, call(None, None, None), ready(Err(failure))).await;
    assert_eq!(
        outcome.expect_err("the handler error comes back"),
        FakeHandlerError::Rejected {
            detail: "denied".to_owned()
        }
    );
    assert!(harness.alert.sent.entries().is_empty());
    assert!(harness.cache.deletes.entries().is_empty());
    assert!(harness.span.sent.entries().is_empty());
}

#[tokio::test]
async fn span_failure_original_error() {
    let (log, _guard) = capture_logs();
    let mut harness = Harness::default();
    harness.span.fails = true;
    let auth = actor_auth();
    let parent = parent_span();
    let failure = ManagementFailure {
        exception: exception(),
        error: FakeHandlerError::Failed,
    };
    let outcome = run(
        &harness,
        call(None, Some(&auth), Some(&parent)),
        ready(Err(failure)),
    )
    .await;
    assert_eq!(
        outcome.expect_err("the handler error comes back"),
        FakeHandlerError::Failed
    );
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.contains("DEBUG"));
    assert!(
        written.contains("Error emitting OTEL span in management endpoint wrapper failure path:")
    );
    assert!(!written.contains("WARN"));
    assert!(!written.contains("ERROR"));
}

#[tokio::test]
async fn customer_cache_eviction() {
    let harness = Harness::default();
    let auth = actor_auth();
    let updated = ManagementData::UpdateCustomer("c1");
    let outcome = run(
        &harness,
        call(Some(&updated), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    outcome.unwrap();
    let deleted = keys(&["c2", "c3"]);
    let data = ManagementData::DeleteCustomers(&deleted);
    let second = run(
        &harness,
        call(Some(&data), Some(&auth), None),
        ready(Ok(created_key())),
    )
    .await;
    second.unwrap();
    assert_eq!(harness.cache.deletes.entries(), keys(&["c1", "c2", "c3"]));
}
