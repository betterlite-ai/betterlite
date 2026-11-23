#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheSettings};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::Number;
use tokio::runtime::{Builder, Runtime};

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_lookups::access_groups::AccessGroups;
use crate::model_group_lookups::error::AccessGroupError;
use crate::model_group_usage::access_filter::{
    AccessFilterRequest, KeyGrants, filter_deployments_by_model_access_groups,
};
use crate::model_group_usage::counters::UsageCounterCache;
use crate::model_group_usage::error::GroupUsageError;
use crate::model_group_usage::metadata::GroupMetadata;
use crate::model_group_usage::remaining::get_remaining_model_group_usage;
use crate::model_group_usage::response_headers::{ResponseHeadersRequest, set_response_headers};
use crate::model_group_usage::usage::{
    IoUsage, Usage, get_model_group_io_token_usage, get_model_group_usage,
};
use crate::remaining_usage_headers::RemainingUsage;
use crate::router_core::headers::Response;
use crate::routing_groups::index::RoutingGroupIndex;

const NOW_SECONDS: u64 = 3660;

#[derive(Clone, Debug, thiserror::Error)]
#[error("the counter read failed")]
struct CacheFailure;

#[derive(Clone, Debug, thiserror::Error)]
#[error("the group metadata read failed")]
struct MetadataFailure;

struct FakeCache {
    answer: Result<Option<Vec<Option<Json>>>, CacheFailure>,
    batches: Mutex<Vec<Vec<String>>>,
}

impl FakeCache {
    fn answering(values: Option<Vec<Option<Json>>>) -> Self {
        Self {
            answer: Ok(values),
            batches: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            answer: Err(CacheFailure),
            batches: Mutex::new(Vec::new()),
        }
    }

    fn batches(&self) -> Vec<Vec<String>> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl UsageCounterCache for FakeCache {
    type Error = CacheFailure;

    async fn batch_get(&self, keys: &[String]) -> Result<Option<Vec<Option<Json>>>, CacheFailure> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(keys.to_vec());
        self.answer.clone()
    }
}

struct FakeMetadata {
    answer: Option<ModelGroupInfo>,
}

#[async_trait]
impl GroupMetadata for FakeMetadata {
    type Error = MetadataFailure;

    async fn cached_group_info(
        &self,
        _model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, MetadataFailure> {
        Ok(self.answer.clone())
    }
}

struct FailingMetadata;

#[async_trait]
impl GroupMetadata for FailingMetadata {
    type Error = MetadataFailure;

    async fn cached_group_info(
        &self,
        _model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, MetadataFailure> {
        Err(MetadataFailure)
    }
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .expect("the test clock")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn answered(values: Vec<Json>) -> Option<Vec<Option<Json>>> {
    Some(values.into_iter().map(Some).collect())
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the fixture float"))
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn generated_id() -> String {
    "generated".to_owned()
}

fn deployment(group: &str, id: &str, model: &str) -> DeploymentSpec {
    DeploymentSpec::new(
        group.to_owned(),
        object(vec![("model", text(model))]),
        object(vec![("id", text(id))]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

fn grouped(group: &str, id: &str, groups: &[&str]) -> DeploymentSpec {
    let listed: Vec<Json> = groups.iter().map(|group| text(group)).collect();
    DeploymentSpec::new(
        group.to_owned(),
        object(vec![("model", text("openai/m"))]),
        object(vec![
            ("id", text(id)),
            ("access_groups", Json::List(listed)),
        ]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

fn group_info(
    tpm: Option<i64>,
    rpm: Option<i64>,
    itpm: Option<i64>,
    otpm: Option<i64>,
) -> ModelGroupInfo {
    ModelGroupInfo {
        model_group: "group".to_owned(),
        providers: Vec::new(),
        max_input_tokens: None,
        max_output_tokens: None,
        input_cost_per_token: None,
        output_cost_per_token: None,
        input_cost_per_pixel: None,
        mode: None,
        tpm,
        rpm,
        itpm,
        otpm,
        capabilities: BTreeSet::new(),
        supports_fast_mode: false,
        supported_reasoning_efforts: None,
        supported_openai_params: None,
        clientside_auth: None,
    }
}

fn grants(models: &[&str], team_models: &[&str]) -> KeyGrants {
    KeyGrants {
        models: models.iter().map(|model| (*model).to_owned()).collect(),
        team_models: team_models
            .iter()
            .map(|model| (*model).to_owned())
            .collect(),
    }
}

const fn no_inferred(_model: &str) -> Option<String> {
    None
}

struct World {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
}

impl World {
    fn new(rows: &[DeploymentSpec]) -> Self {
        let mut world = Self {
            catalog: DeploymentCatalog::default(),
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
        };
        for row in rows {
            world
                .catalog
                .index_patterns(row)
                .expect("the fixture pattern index");
            world.catalog.push(row.clone());
        }
        world
    }

    fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &no_inferred,
        }
    }
}

fn tpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:tpm:01-01")
}

fn rpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:rpm:01-01")
}

fn itpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:itpm:01-01")
}

fn otpm_key(id: &str, model: &str) -> String {
    format!("global_router:{id}:{model}:otpm:01-01")
}

fn ordered(headers: &RemainingUsage) -> Vec<(&str, i64)> {
    headers
        .iter()
        .map(|(header, value)| (header.as_str(), *value))
        .collect()
}

#[test]
fn remaining_rpm_reads_both_sides_and_reports_the_request_headers() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o-mini"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        number(10),
        number(30),
    ]));
    let metadata = FakeMetadata {
        answer: Some(group_info(None, Some(100), None, None)),
    };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-remaining-requests", 60_i64),
            ("x-ratelimit-limit-requests", 100_i64),
        ]
    );
    assert_eq!(
        cache.batches(),
        [vec![
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("b", "openai/gpt-4o-mini"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("b", "openai/gpt-4o-mini"),
        ]]
    );
}

#[test]
fn remaining_tpm_reports_the_token_headers_only() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(20), number(3)]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(50), None, None, None)),
    };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-remaining-tokens", 30_i64),
            ("x-ratelimit-limit-tokens", 50_i64),
        ]
    );
}

#[test]
fn input_output_usage_builds_the_io_headers_from_the_io_counters() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(5), number(7)]));
    let metadata = FakeMetadata {
        answer: Some(group_info(None, None, Some(40), Some(30))),
    };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-limit-input-tokens", 40_i64),
            ("x-ratelimit-remaining-input-tokens", 35_i64),
            ("x-ratelimit-reset-input-tokens", 1_i64),
            ("x-ratelimit-limit-output-tokens", 30_i64),
            ("x-ratelimit-remaining-output-tokens", 23_i64),
            ("x-ratelimit-reset-output-tokens", 1_i64),
        ]
    );
    assert_eq!(
        cache.batches(),
        [vec![
            itpm_key("a", "openai/gpt-4o"),
            otpm_key("a", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn missing_counter_counts_as_zero_usage() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![Json::Null, Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(100), Some(10), None, None)),
    };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(headers.get("x-ratelimit-remaining-tokens"), Some(&100_i64));
    assert_eq!(headers.get("x-ratelimit-remaining-requests"), Some(&10_i64));
}

#[test]
fn response_headers_stamp_the_model_id_group_and_remaining_usage() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(3), Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(10), None, None, None)),
    };
    let kwargs = object(vec![("metadata", bucket("a"))]);
    let response = Response::Object(object(vec![("content", text("hi"))]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: Some("group"),
            request_kwargs: Some(&kwargs),
            model_list: Some(&rows),
            metadata: &metadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(hidden(&returned).get("model_id"), Some(&text("a")));
    let headers = headers(&returned);
    assert_eq!(headers.get("x-litellm-model-group"), Some(&text("group")));
    assert_eq!(
        headers.get("x-ratelimit-remaining-tokens"),
        Some(&number(7))
    );
    assert_eq!(headers.get("x-ratelimit-limit-tokens"), Some(&number(10)));
}

fn bucket(id: &str) -> Json {
    Json::Object(object(vec![(
        "model_info",
        Json::Object(object(vec![("id", text(id))])),
    )]))
}

fn hidden(response: &Response) -> &JsonObject {
    match response {
        Response::Object(body) => match body.get("_hidden_params") {
            Some(Json::Object(hidden)) => hidden,
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                panic!("the response carries no hidden params")
            }
        },
        Response::Absent | Response::Attributed(_) | Response::Stream | Response::Opaque => {
            panic!("the response is not a dict")
        }
    }
}

fn headers(response: &Response) -> &JsonObject {
    match hidden(response).get("additional_headers") {
        Some(Json::Object(headers)) => headers,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            panic!("the response has no header map")
        }
    }
}

fn filtered_ids(
    request: AccessFilterRequest<'_>,
    access_groups: &AccessGroups,
    world: &World,
) -> Vec<String> {
    let lookup = world.lookup();
    filter_deployments_by_model_access_groups(request, access_groups, &lookup)
        .expect("the access filter")
        .iter()
        .map(|deployment| deployment.id().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn access_group_filter_keeps_the_candidates_in_the_callers_groups() {
    let rows = [
        grouped("group", "a", &["alpha", "beta"]),
        grouped("group", "b", &["beta"]),
        grouped("group", "c", &["gamma"]),
    ];
    let world = World::new(&rows);
    let grants = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&grants),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn batch_key_order_sends_every_tpm_key_before_every_rpm_key() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        Json::Null,
        Json::Null,
    ]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert_eq!(
        cache.batches(),
        [vec![
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("b", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("b", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn io_batch_key_order_sends_every_itpm_key_before_every_otpm_key() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        Json::Null,
        Json::Null,
    ]));
    let usage = runtime()
        .block_on(get_model_group_io_token_usage(Some(&rows), &cache, now()))
        .expect("the io usage read");
    assert_eq!(usage, IoUsage::default());
    assert_eq!(
        cache.batches(),
        [vec![
            itpm_key("a", "openai/gpt-4o"),
            itpm_key("b", "openai/gpt-4o"),
            otpm_key("a", "openai/gpt-4o"),
            otpm_key("b", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn skip_missing_identity_reads_only_the_identified_deployments() {
    let mut missing_id = deployment("group", "gone", "openai/gpt-4o");
    missing_id.model_info.shift_remove("id");
    let mut empty_id = deployment("group", "", "openai/gpt-4o");
    empty_id.model_info.insert("id".to_owned(), text(""));
    let mut missing_model = deployment("group", "b", "openai/gpt-4o");
    missing_model.litellm_params.shift_remove("model");
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        missing_id,
        empty_id,
        missing_model,
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        Json::Null,
        Json::Null,
    ]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert_eq!(
        cache.batches(),
        [vec![
            tpm_key("a", "openai/gpt-4o"),
            "global_router:::tpm:01-01".to_owned(),
            rpm_key("a", "openai/gpt-4o"),
            "global_router:::rpm:01-01".to_owned(),
        ]]
    );
}

#[test]
fn empty_batch_reads_once_with_no_keys() {
    let rows: [DeploymentSpec; 0] = [];
    let cache = FakeCache::answering(Some(Vec::new()));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert_eq!(cache.batches(), [Vec::new()]);
}

#[test]
fn null_model_list_skips_the_cache() {
    let cache = FakeCache::failing();
    let usage = runtime()
        .block_on(get_model_group_usage(None, &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert!(cache.batches().is_empty());
}

#[rstest]
#[case::shorter_than_the_first_side(vec![Some(number(5))], (Some(5), None))]
#[case::split_across_a_short_answer(
    vec![Some(number(5)), Some(number(7)), Some(number(11))],
    (Some(12), Some(11))
)]
#[case::the_whole_answer(
    vec![
        Some(number(5)),
        Some(number(7)),
        Some(number(11)),
        Some(number(13)),
    ],
    (Some(12), Some(24))
)]
#[case::no_values(vec![], (None, None))]
fn short_batch_keeps_the_source_order(
    #[case] values: Vec<Option<Json>>,
    #[case] expected: (Option<i64>, Option<i64>),
) {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(Some(values));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!((usage.tpm, usage.rpm), expected);
}

#[test]
fn boolean_and_invalid_counters_sum_only_python_ints() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![
        Json::Bool(true),
        float(1.5),
        number(2),
        text("3"),
        Json::Bool(false),
        Json::List(Vec::new()),
        Json::Null,
        Json::Object(JsonObject::new()),
    ]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(
        usage,
        Usage {
            tpm: Some(3),
            rpm: Some(0)
        }
    );
}

#[rstest]
#[case::a_zero_counter_is_usage(vec![Some(number(0)), Some(Json::Null)], (Some(0), None))]
#[case::no_counters_is_none(vec![Some(Json::Null), Some(Json::Null)], (None, None))]
#[case::the_rpm_side_can_be_zero_too(vec![Some(Json::Null), Some(number(0))], (None, Some(0)))]
fn zero_versus_none(
    #[case] values: Vec<Option<Json>>,
    #[case] expected: (Option<i64>, Option<i64>),
) {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(Some(values));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!((usage.tpm, usage.rpm), expected);
}

#[test]
fn duplicate_deployment_sum_counts_every_key_in_the_batch() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "a", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![number(1), number(2), number(3), number(4)]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(
        usage,
        Usage {
            tpm: Some(3),
            rpm: Some(7)
        }
    );
    let batches = cache.batches();
    let first = batches.first().expect("the batch read");
    assert_eq!(
        first,
        &[
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("a", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
        ]
    );
}

#[test]
fn io_and_standard_usage_are_read_separately_and_merged() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(5), number(7)]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(100), Some(50), Some(40), Some(30))),
    };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-limit-input-tokens", 40_i64),
            ("x-ratelimit-remaining-input-tokens", 35_i64),
            ("x-ratelimit-reset-input-tokens", 1_i64),
            ("x-ratelimit-limit-output-tokens", 30_i64),
            ("x-ratelimit-remaining-output-tokens", 23_i64),
            ("x-ratelimit-reset-output-tokens", 1_i64),
            ("x-ratelimit-remaining-tokens", 95_i64),
            ("x-ratelimit-limit-tokens", 100_i64),
            ("x-ratelimit-remaining-requests", 43_i64),
            ("x-ratelimit-limit-requests", 50_i64),
        ]
    );
    assert_eq!(
        cache.batches(),
        [
            vec![
                itpm_key("a", "openai/gpt-4o"),
                otpm_key("a", "openai/gpt-4o"),
            ],
            vec![tpm_key("a", "openai/gpt-4o"), rpm_key("a", "openai/gpt-4o"),],
        ]
    );
}

#[test]
fn ordinary_remaining_may_go_negative() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(25), Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(10), None, None, None)),
    };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert_eq!(headers.get("x-ratelimit-remaining-tokens"), Some(&-15_i64));
}

#[test]
fn io_remaining_clamps_at_zero_and_the_reset_is_whole() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(25), Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(None, None, Some(10), None)),
    };
    let start = UNIX_EPOCH
        .checked_add(Duration::from_secs(60))
        .expect("the minute edge");
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            start,
        ))
        .expect("the remaining usage");
    assert_eq!(
        ordered(&headers),
        [
            ("x-ratelimit-limit-input-tokens", 10_i64),
            ("x-ratelimit-remaining-input-tokens", 0_i64),
            ("x-ratelimit-reset-input-tokens", 60_i64),
        ]
    );
}

#[rstest]
#[case::unknown_group(None)]
#[case::no_limits(Some(group_info(None, None, None, None)))]
fn no_group_or_limits_reads_no_counters(#[case] answer: Option<ModelGroupInfo>) {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::failing();
    let metadata = FakeMetadata { answer };
    let headers = runtime()
        .block_on(get_remaining_model_group_usage(
            "group",
            Some(&rows),
            &metadata,
            &cache,
            now(),
        ))
        .expect("the remaining usage");
    assert!(headers.is_empty());
    assert!(cache.batches().is_empty());
}

#[test]
fn metadata_failure_precedes_the_counter_read() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(3), Json::Null]));
    let outcome = runtime().block_on(get_remaining_model_group_usage(
        "group",
        Some(&rows),
        &FailingMetadata,
        &cache,
        now(),
    ));
    assert!(matches!(
        outcome,
        Err(GroupUsageError::Metadata(MetadataFailure))
    ));
    assert!(cache.batches().is_empty());
}

#[test]
fn batch_error_surfaces_from_both_usage_reads() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::failing();
    let standard = runtime().block_on(get_model_group_usage(Some(&rows), &cache, now()));
    assert!(matches!(standard, Err(CacheFailure)));
    let io = runtime().block_on(get_model_group_io_token_usage(Some(&rows), &cache, now()));
    assert!(matches!(io, Err(CacheFailure)));
}

#[test]
fn null_response_is_returned_untouched_before_any_read() {
    let cache = FakeCache::failing();
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Absent,
            model_group: Some("group"),
            request_kwargs: None,
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the untouched response");
    assert_eq!(returned, Response::Absent);
    assert!(cache.batches().is_empty());
}

#[test]
fn null_group_still_stamps_the_group_header() {
    let cache = FakeCache::failing();
    let response = Response::Object(object(vec![("content", text("hi"))]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: None,
            request_kwargs: None,
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(
        headers(&returned),
        &object(vec![("x-litellm-model-group", Json::Null)])
    );
    assert!(cache.batches().is_empty());
}

#[test]
fn model_id_prefers_litellm_metadata_then_keeps_an_existing_one() {
    let cache = FakeCache::failing();
    let both = object(vec![
        ("litellm_metadata", bucket("litellm")),
        ("metadata", bucket("metadata")),
    ]);
    let only_metadata = object(vec![("metadata", bucket("metadata"))]);
    let existing = Response::Object(object(vec![(
        "_hidden_params",
        Json::Object(object(vec![("model_id", text("existing"))])),
    )]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: existing,
            model_group: None,
            request_kwargs: Some(&both),
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(hidden(&returned).get("model_id"), Some(&text("existing")));
    let fresh = Response::Object(JsonObject::new());
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: fresh,
            model_group: None,
            request_kwargs: Some(&both),
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(hidden(&returned).get("model_id"), Some(&text("litellm")));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Object(JsonObject::new()),
            model_group: None,
            request_kwargs: Some(&only_metadata),
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(hidden(&returned).get("model_id"), Some(&text("metadata")));
}

#[test]
fn existing_rate_limit_headers_win_over_the_remaining_usage() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(answered(vec![number(4), Json::Null]));
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(100), None, None, None)),
    };
    let response = Response::Object(object(vec![(
        "_hidden_params",
        Json::Object(object(vec![(
            "additional_headers",
            Json::Object(object(vec![("x-ratelimit-limit-tokens", number(5))])),
        )])),
    )]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: Some("group"),
            request_kwargs: None,
            model_list: Some(&rows),
            metadata: &metadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    let headers = headers(&returned);
    assert_eq!(headers.get("x-ratelimit-limit-tokens"), Some(&number(5)));
    assert_eq!(
        headers.get("x-ratelimit-remaining-tokens"),
        Some(&number(96))
    );
    assert_eq!(headers.get("x-litellm-model-group"), Some(&text("group")));
}

#[test]
fn decision_headers_carry_the_quality_and_complexity_decisions() {
    let cache = FakeCache::failing();
    let kwargs = object(vec![
        (
            "metadata",
            Json::Object(object(vec![(
                "quality_router_decision",
                Json::Object(object(vec![
                    ("routed_model", text("m")),
                    ("quality_tier", text("tier-one")),
                ])),
            )])),
        ),
        (
            "litellm_metadata",
            Json::Object(object(vec![(
                "routing_decision",
                Json::Object(object(vec![
                    ("router_type", text("complexity")),
                    ("tier", text("T2")),
                ])),
            )])),
        ),
    ]);
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Object(JsonObject::new()),
            model_group: None,
            request_kwargs: Some(&kwargs),
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    let headers = headers(&returned);
    assert_eq!(
        headers.get("x-litellm-quality-router-model"),
        Some(&text("m"))
    );
    assert_eq!(
        headers.get("x-litellm-quality-router-tier"),
        Some(&text("tier-one"))
    );
    assert_eq!(
        headers.get("x-litellm-complexity-router-tier"),
        Some(&text("T2"))
    );
}

#[test]
fn auth_metadata_precedence_prefers_the_request_metadata_principal() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["beta"]),
    ];
    let world = World::new(&rows);
    let direct = grants(&["group"], &[]);
    let grouped_only = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&direct),
        litellm_metadata: Some(&grouped_only),
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: None,
        litellm_metadata: Some(&grouped_only),
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["b"]
    );
}

#[test]
fn no_principal_or_grants_leaves_the_candidates_untouched() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["beta"]),
    ];
    let world = World::new(&rows);
    let empty = KeyGrants::default();
    for request in [
        AccessFilterRequest {
            model: "group",
            deployments: &rows,
            metadata: None,
            litellm_metadata: None,
            team_id: None,
        },
        AccessFilterRequest {
            model: "group",
            deployments: &rows,
            metadata: Some(&empty),
            litellm_metadata: None,
            team_id: None,
        },
    ] {
        assert_eq!(
            filtered_ids(request, &AccessGroups::default(), &world),
            ["a", "b"]
        );
    }
}

#[rstest]
#[case::the_model_itself(&["group"])]
#[case::the_wildcard(&["*"])]
#[case::the_proxy_wildcard(&["all-proxy-models"])]
#[case::a_team_grant(&[])]
fn direct_and_wildcard_grants_bypass_the_filter(#[case] models: &[&str]) {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["beta"]),
    ];
    let world = World::new(&rows);
    let team = if models.is_empty() {
        vec!["group"]
    } else {
        Vec::new()
    };
    let principal = KeyGrants {
        models: models.iter().map(|model| (*model).to_owned()).collect(),
        team_models: team.iter().map(|model| (*model).to_owned()).collect(),
    };
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn no_group_overlap_leaves_the_candidates_untouched() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["alpha"]),
    ];
    let world = World::new(&rows);
    let principal = grants(&["delta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn group_only_result_can_be_empty() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "c", &["gamma"]),
    ];
    let world = World::new(&rows);
    let candidates = [rows.first().expect("the first candidate").clone()];
    let principal = grants(&["gamma"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &candidates,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert!(filtered_ids(request, &AccessGroups::default(), &world).is_empty());
}

#[test]
fn an_unlisted_access_groups_value_is_rejected() {
    let mut broken = grouped("group", "a", &["beta"]);
    broken
        .model_info
        .insert("access_groups".to_owned(), text("beta"));
    let world = World::new(&[grouped("group", "a", &["beta"])]);
    let principal = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &[broken],
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    let lookup = world.lookup();
    let outcome =
        filter_deployments_by_model_access_groups(request, &AccessGroups::default(), &lookup);
    assert!(matches!(
        outcome,
        Err(AccessGroupError::InvalidAccessGroups { model_name }) if model_name == "group"
    ));
}

#[test]
fn reads_counters_from_the_dual_cache() {
    let memory =
        LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds");
    let cache: DualCache<LocalMemoryOps> = DualCache::new(
        Arc::new(memory),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
    );
    cache.set_cache(
        &tpm_key("a", "openai/gpt-4o"),
        &number(7),
        CacheOptions::default(),
    );
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(
        usage,
        Usage {
            tpm: Some(7),
            rpm: None
        }
    );
}
