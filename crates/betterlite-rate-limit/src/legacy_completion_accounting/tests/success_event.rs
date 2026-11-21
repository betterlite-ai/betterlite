use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::deployment::{
    DeploymentLimitCatalog, DeploymentLimitValue, DeploymentLimits,
};
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use serde_json::Number;

use super::support::{BATCH_WRITE, Call, Fixture, MINUTE, counter, fixture, minute_key};
use crate::legacy_completion_accounting::response_tokens::{CompletionResponse, ResponseUsage};
use crate::legacy_completion_accounting::success_event::{
    SuccessEvent, SuccessMetadata, UserApiKeyField,
};
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::LegacyCacheValue;

const GROUP_KEY: &str = "sk-key::gpt-4o::2026-11-21-09-05::request_count";

// one deployment of every model group with this default_api_key_tpm_limit
struct DefaultTpm(DeploymentLimitValue);

impl DeploymentLimitCatalog for DefaultTpm {
    fn model_list(&self, _model_name: &str) -> Option<Vec<DeploymentLimits>> {
        Some(vec![DeploymentLimits {
            default_api_key_rpm_limit: None,
            default_api_key_tpm_limit: Some(self.0.clone()),
        }])
    }
}

fn metadata(user_api_key: UserApiKeyField) -> SuccessMetadata {
    SuccessMetadata {
        global_max_parallel_requests: None,
        user_api_key,
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_model_max_budget: None,
        user_api_key_metadata: BTreeMap::new(),
        user_api_key_team_metadata: None,
    }
}

fn event<'event>(metadata: SuccessMetadata) -> SuccessEvent<'event> {
    SuccessEvent {
        metadata,
        end_user: None,
        model_group: None,
        response: CompletionResponse::Chat(Some(ResponseUsage { total_tokens: 10 })),
        limit_catalog: None,
        parent_otel_span: None,
    }
}

fn key() -> UserApiKeyField {
    UserApiKeyField::Key(String::from("sk-key"))
}

fn limits(entries: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn per_model(limit: i64) -> JsonData {
    JsonData::Object(limits(&[("gpt-4o", JsonData::Number(Number::from(limit)))]))
}

async fn written(fixture: &Fixture, event: &SuccessEvent<'_>) -> Vec<(String, LegacyCacheValue)> {
    let counters = fixture.cache.as_ref();
    fixture.handler.log_success_event(counters, event).await;
    let calls = fixture.cache.calls();
    let Some(Call::BatchSet { entries, .. }) = calls.last() else {
        panic!("the last call is not the batch write: {calls:?}");
    };
    entries.clone()
}

#[rstest]
#[case::limit_set(Some(10), true)]
#[case::limit_zero(Some(0), true)]
#[case::limit_unset(None, false)]
#[tokio::test]
async fn global_slot_released_local_only_when_the_limit_is_set(
    #[case] global_limit: Option<i64>,
    #[case] released: bool,
) {
    let fixture = fixture(&[], &[]);
    let mut success = metadata(key());
    success.global_max_parallel_requests = global_limit;
    fixture
        .handler
        .log_success_event(fixture.cache.as_ref(), &event(success))
        .await;
    let calls = fixture.cache.calls();
    let decrement = Call::Increment {
        key: String::from("global_max_parallel_requests"),
        delta: -1,
        local_only: true,
    };
    assert_eq!(calls.first() == Some(&decrement), released);
    assert_eq!(
        calls.iter().filter(|call| **call == decrement).count(),
        usize::from(released)
    );
}

#[rstest]
#[case::cache_miss(None, counter(0, 10, 0))]
#[case::existing(Some(counter(3, 100, 5)), counter(2, 110, 5))]
#[case::floor_at_zero(Some(counter(0, 7, 2)), counter(0, 17, 2))]
#[tokio::test]
async fn key_counter_releases_the_slot_and_adds_tokens(
    #[case] stored: Option<LegacyCacheValue>,
    #[case] expected: LegacyCacheValue,
) {
    let key_counter = minute_key("sk-key");
    let stored_entries: Vec<(&str, LegacyCacheValue)> = stored
        .map(|value| (key_counter.as_str(), value))
        .into_iter()
        .collect();
    let fixture = fixture(&stored_entries, &[]);
    let entries = written(&fixture, &event(metadata(key()))).await;
    assert_eq!(entries, vec![(key_counter.clone(), expected)]);
    let LegacyCacheValue::Counter(new_val) = expected else {
        panic!("the expectation is a counter");
    };
    assert_eq!(
        fixture.console.lines(),
        vec![
            String::from("INSIDE parallel request limiter ASYNC SUCCESS LOGGING"),
            format!("updated_value in success call: {new_val:?}, precise_minute: {MINUTE}"),
        ]
    );
}

#[tokio::test]
async fn key_counter_skipped_for_a_null_key() {
    let fixture = fixture(&[], &[]);
    let mut success = metadata(UserApiKeyField::Null);
    success.user_api_key_metadata = limits(&[("model_rpm_limit", per_model(5))]);
    let mut null_key = event(success);
    null_key.model_group = Some(String::from("gpt-4o"));
    assert_eq!(written(&fixture, &null_key).await, Vec::new());
}

#[rstest]
#[case::rpm_key_present(&[("model_rpm_limit", JsonData::Null)], None, None, true)]
#[case::tpm_key_present(&[("model_tpm_limit", JsonData::String(String::new()))], None, None, true)]
#[case::empty_model_max_budget(&[], Some(BTreeMap::new()), None, true)]
#[case::team_tpm_lookup(&[], None, Some(limits(&[("model_tpm_limit", per_model(5))])), true)]
#[case::team_rpm_lookup(&[], None, Some(limits(&[("model_rpm_limit", per_model(5))])), true)]
#[case::no_limit_source(&[("other", JsonData::Null)], None, Some(BTreeMap::new()), false)]
#[tokio::test]
async fn model_group_written_only_with_a_limit_source(
    #[case] key_metadata: &[(&str, JsonData)],
    #[case] model_max_budget: Option<BTreeMap<String, JsonData>>,
    #[case] team_metadata: Option<BTreeMap<String, JsonData>>,
    #[case] counted: bool,
) {
    let fixture = fixture(&[], &[]);
    let mut success = metadata(key());
    success.user_api_key_metadata = limits(key_metadata);
    success.user_api_key_model_max_budget = model_max_budget;
    success.user_api_key_team_metadata = team_metadata;
    let mut grouped = event(success);
    grouped.model_group = Some(String::from("gpt-4o"));
    let entries = written(&fixture, &grouped).await;
    let mut expected = vec![(minute_key("sk-key"), counter(0, 10, 0))];
    if counted {
        expected.push((String::from(GROUP_KEY), counter(0, 10, 0)));
    }
    assert_eq!(entries, expected);
}

#[rstest]
#[case::catalog_default(Some(DeploymentLimitValue::Integer(50)), true)]
#[case::catalog_without_default(Some(DeploymentLimitValue::Unsupported), false)]
#[case::no_catalog(None, false)]
#[tokio::test]
async fn model_group_counts_a_deployment_default_limit(
    #[case] default_tpm: Option<DeploymentLimitValue>,
    #[case] counted: bool,
) {
    let fixture = fixture(&[(GROUP_KEY, counter(4, 50, 6))], &[]);
    let catalog = default_tpm.map(DefaultTpm);
    let mut grouped = event(metadata(key()));
    grouped.model_group = Some(String::from("gpt-4o"));
    if let Some(source) = &catalog {
        grouped.limit_catalog = Some(source);
    }
    let entries = written(&fixture, &grouped).await;
    let model_entry = (String::from(GROUP_KEY), counter(3, 60, 6));
    assert_eq!(entries.contains(&model_entry), counted);
}

#[tokio::test]
async fn model_group_skipped_without_a_group() {
    let fixture = fixture(&[], &[]);
    let mut success = metadata(key());
    success.user_api_key_metadata = limits(&[("model_rpm_limit", per_model(5))]);
    let entries = written(&fixture, &event(success)).await;
    assert_eq!(entries, vec![(minute_key("sk-key"), counter(0, 10, 0))]);
}

#[rstest]
#[case::user_miss(Some("user-1"), None, None, None, counter(0, 20, 1))]
#[case::team_miss(None, Some("team-1"), None, None, counter(0, 20, 1))]
#[case::end_user_miss(None, None, Some("end-1"), None, counter(0, 20, 1))]
#[case::user_existing(Some("user-1"), None, None, Some(counter(2, 30, 3)), counter(1, 40, 3))]
#[case::team_floor(None, Some("team-1"), None, Some(counter(0, 5, 4)), counter(0, 15, 4))]
#[tokio::test]
async fn entity_counter_seeds_a_miss_with_the_tokens(
    #[case] user_id: Option<&str>,
    #[case] team_id: Option<&str>,
    #[case] end_user: Option<&str>,
    #[case] stored: Option<LegacyCacheValue>,
    #[case] expected: LegacyCacheValue,
) {
    let id = user_id.or(team_id).or(end_user).unwrap();
    let entity_key = minute_key(id);
    let stored_entries: Vec<(&str, LegacyCacheValue)> = stored
        .map(|value| (entity_key.as_str(), value))
        .into_iter()
        .collect();
    let fixture = fixture(&stored_entries, &[]);
    let mut success = metadata(UserApiKeyField::Null);
    success.user_api_key_user_id = user_id.map(str::to_owned);
    success.user_api_key_team_id = team_id.map(str::to_owned);
    let mut entity = event(success);
    entity.end_user = end_user.map(str::to_owned);
    assert_eq!(
        written(&fixture, &entity).await,
        vec![(entity_key.clone(), expected)]
    );
}

#[tokio::test]
async fn batch_write_sends_every_counter_once_in_block_order() {
    let fixture = fixture(&[], &[]);
    let mut success = metadata(key());
    success.user_api_key_user_id = Some(String::from("user-1"));
    success.user_api_key_team_id = Some(String::from("team-1"));
    success.user_api_key_model_max_budget = Some(BTreeMap::new());
    let mut everything = event(success);
    everything.end_user = Some(String::from("end-1"));
    everything.model_group = Some(String::from("gpt-4o"));
    fixture
        .handler
        .log_success_event(fixture.cache.as_ref(), &everything)
        .await;
    let keys = [
        minute_key("sk-key"),
        String::from(GROUP_KEY),
        minute_key("user-1"),
        minute_key("team-1"),
        minute_key("end-1"),
    ];
    let mut expected: Vec<Call> = keys
        .iter()
        .map(|key| Call::Get {
            key: key.clone(),
            local_only: false,
        })
        .collect();
    expected.push(Call::BatchSet {
        entries: vec![
            (keys[0].clone(), counter(0, 10, 0)),
            (keys[1].clone(), counter(0, 10, 0)),
            (keys[2].clone(), counter(0, 20, 1)),
            (keys[3].clone(), counter(0, 20, 1)),
            (keys[4].clone(), counter(0, 20, 1)),
        ],
        ttl: 60,
        local_only: false,
    });
    assert_eq!(fixture.cache.calls(), expected);
}

#[tokio::test]
async fn batch_write_happens_with_an_empty_list() {
    let fixture = fixture(&[], &[]);
    fixture
        .handler
        .log_success_event(
            fixture.cache.as_ref(),
            &event(metadata(UserApiKeyField::Null)),
        )
        .await;
    assert_eq!(
        fixture.cache.calls(),
        vec![Call::BatchSet {
            entries: Vec::new(),
            ttl: 60,
            local_only: false,
        }]
    );
}

#[tokio::test]
async fn swallowed_error_missing_key_writes_nothing() {
    let fixture = fixture(&[], &[]);
    let mut success = metadata(UserApiKeyField::Missing);
    success.global_max_parallel_requests = Some(10);
    success.user_api_key_user_id = Some(String::from("user-1"));
    fixture
        .handler
        .log_success_event(fixture.cache.as_ref(), &event(success))
        .await;
    assert_eq!(fixture.cache.calls(), Vec::new());
    assert_eq!(
        fixture.console.lines().last().map(String::as_str),
        Some("the request metadata has no user_api_key")
    );
}

#[rstest]
#[case::bare_number_at_key(&[("sk-key::2026-11-21-09-05::request_count", LegacyCacheValue::Count(3))], &[], "sk-key::2026-11-21-09-05::request_count holds a bare number, not a request counter")]
#[case::failed_read(&[], &["user-1::2026-11-21-09-05::request_count"], "the usage cache call for user-1::2026-11-21-09-05::request_count failed")]
#[case::token_overflow(&[("sk-key::2026-11-21-09-05::request_count", counter(1, i64::MAX, 0))], &[], "the counter under sk-key::2026-11-21-09-05::request_count does not fit a 64-bit integer")]
#[case::request_overflow(&[("sk-key::2026-11-21-09-05::request_count", counter(i64::MIN, 0, 0))], &[], "the counter under sk-key::2026-11-21-09-05::request_count does not fit a 64-bit integer")]
#[case::failed_batch_write(&[], &[BATCH_WRITE], "the batch write of the new counters failed")]
#[tokio::test]
async fn swallowed_error_keeps_the_global_release(
    #[case] stored: &[(&str, LegacyCacheValue)],
    #[case] failing: &[&str],
    #[case] printed: &str,
) {
    let fixture = fixture(stored, failing);
    let mut success = metadata(key());
    success.global_max_parallel_requests = Some(10);
    success.user_api_key_user_id = Some(String::from("user-1"));
    fixture
        .handler
        .log_success_event(fixture.cache.as_ref(), &event(success))
        .await;
    let calls = fixture.cache.calls();
    assert_eq!(
        calls.first(),
        Some(&Call::Increment {
            key: String::from("global_max_parallel_requests"),
            delta: -1,
            local_only: true,
        })
    );
    let batch_writes = calls
        .iter()
        .filter(|call| matches!(call, Call::BatchSet { .. }))
        .count();
    assert_eq!(batch_writes, usize::from(failing == [BATCH_WRITE]));
    assert_eq!(
        fixture.console.lines().last().map(String::as_str),
        Some(printed)
    );
}

#[tokio::test]
async fn swallowed_error_from_the_limit_lookup_skips_later_blocks() {
    let fixture = fixture(&[], &[]);
    let catalog = DefaultTpm(DeploymentLimitValue::Unsigned(u64::MAX));
    let mut success = metadata(key());
    success.user_api_key_user_id = Some(String::from("user-1"));
    let mut grouped = event(success);
    grouped.model_group = Some(String::from("gpt-4o"));
    grouped.limit_catalog = Some(&catalog);
    fixture
        .handler
        .log_success_event(fixture.cache.as_ref(), &grouped)
        .await;
    assert_eq!(
        fixture.cache.calls(),
        vec![Call::Get {
            key: minute_key("sk-key"),
            local_only: false,
        }]
    );
    assert_eq!(
        fixture.console.lines().last().map(String::as_str),
        Some("the per-model limit lookup failed")
    );
}
