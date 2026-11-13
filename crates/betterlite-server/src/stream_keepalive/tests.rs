#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::settings::JsonValue;
use futures_util::StreamExt as _;
use futures_util::stream;
use rstest::rstest;
use tempfile::NamedTempFile;
use tokio::time::{Duration, Instant, advance, sleep, timeout};
use tracing::subscriber::set_default;

use super::deployment::{DeploymentKeepalive, deployment_keepalive};
use super::interval::is_explicit_keepalive_disable;
use super::resolver::KeepaliveResolver;
use super::wrapper::{Beat, KeepaliveSource, with_keepalive};
use super::{JsonObject, KeepaliveRouter};
use crate::keepalive_interval::PingSetting;

fn map(text: &str) -> JsonObject {
    match value(text) {
        JsonValue::Object(fields) => fields.into_iter().collect(),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => panic!("the fixture is not a JSON object"),
    }
}

fn assert_seconds(actual: f64, expected: f64) {
    assert_eq!(actual.to_bits(), expected.to_bits());
}

fn value(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("valid JSON value")
}

#[derive(Default)]
struct FakeRouter {
    deployments: Mutex<BTreeMap<String, JsonObject>>,
    groups: Mutex<BTreeMap<String, Vec<JsonValue>>>,
    group_names: Mutex<Vec<Option<String>>>,
    deployment_calls: AtomicUsize,
    group_calls: AtomicUsize,
}

impl FakeRouter {
    fn deployment(self, model_id: &str, params: &str) -> Self {
        self.deployments
            .lock()
            .expect("deployment map")
            .insert(model_id.to_owned(), map(params));
        self
    }

    fn group(self, model_name: &str, deployments: &str) -> Self {
        self.groups
            .lock()
            .expect("group map")
            .insert(model_name.to_owned(), parse_list(deployments));
        self
    }

    fn deployment_calls(&self) -> usize {
        self.deployment_calls.load(Ordering::SeqCst)
    }

    fn group_calls(&self) -> usize {
        self.group_calls.load(Ordering::SeqCst)
    }

    fn requested_groups(&self) -> Vec<Option<String>> {
        self.group_names.lock().expect("group names").clone()
    }
}

fn parse_list(text: &str) -> Vec<JsonValue> {
    serde_json::from_str(text).expect("valid JSON list")
}

impl KeepaliveRouter for FakeRouter {
    fn deployment_params(&self, model_id: &str) -> Option<JsonObject> {
        self.deployment_calls.fetch_add(1, Ordering::SeqCst);
        self.deployments
            .lock()
            .expect("deployment map")
            .get(model_id)
            .cloned()
    }

    fn group_deployments(&self, model_name: Option<&str>) -> Vec<JsonValue> {
        self.group_calls.fetch_add(1, Ordering::SeqCst);
        self.group_names
            .lock()
            .expect("group names")
            .push(model_name.map(str::to_owned));
        let Some(name) = model_name else {
            return Vec::new();
        };
        self.groups
            .lock()
            .expect("group map")
            .get(name)
            .cloned()
            .unwrap_or_default()
    }
}

fn fake_router(router: FakeRouter) -> Arc<FakeRouter> {
    Arc::new(router)
}

fn dyn_router(router: Arc<FakeRouter>) -> Arc<dyn KeepaliveRouter> {
    router
}

fn resolver_for(
    router: &Arc<FakeRouter>,
    request: &str,
    global: Option<PingSetting>,
) -> KeepaliveResolver {
    KeepaliveResolver::new(
        Some(dyn_router(Arc::clone(router))),
        Arc::new(map(request)),
        global,
    )
}

fn resolve(
    router: &Arc<FakeRouter>,
    request: &str,
    hidden: &str,
    global: Option<PingSetting>,
) -> f64 {
    let mut resolver = KeepaliveResolver::new(
        Some(dyn_router(Arc::clone(router))),
        Arc::new(map(request)),
        global,
    );
    resolver.resolve(&map(hidden))
}

fn resolve_without_router(request: &str, hidden: &str, global: Option<PingSetting>) -> f64 {
    let mut resolver = KeepaliveResolver::new(None, Arc::new(map(request)), global);
    resolver.resolve(&map(hidden))
}

#[derive(Debug, PartialEq)]
struct FakeChunk {
    hidden: Option<JsonObject>,
}

impl KeepaliveSource for FakeChunk {
    fn hidden_params(&self) -> Option<&JsonObject> {
        self.hidden.as_ref()
    }
}

fn chunk() -> FakeChunk {
    FakeChunk { hidden: None }
}

fn hidden_chunk(model_id: &str) -> FakeChunk {
    FakeChunk {
        hidden: Some(map(&format!(r#"{{"model_id": "{model_id}"}}"#))),
    }
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::zero_text(r#""0""#)]
#[case::padded_negative_text(r#"" -2.5 ""#)]
#[case::negative_infinity_text(r#""-inf""#)]
#[case::underflowed_text(r#""1e-400""#)]
fn explicit_disable_values(#[case] raw: &str) {
    assert!(is_explicit_keepalive_disable(Some(&value(raw))));
}

#[rstest]
#[case::positive("5")]
#[case::positive_text(r#""5""#)]
#[case::unparsable_text(r#""abc""#)]
#[case::nan_text(r#""nan""#)]
#[case::nan_case_text(r#""NaN""#)]
#[case::infinity_text(r#""Infinity""#)]
#[case::null("null")]
#[case::bool_true("true")]
#[case::bool_false("false")]
#[case::object("{}")]
fn not_a_disable(#[case] raw: &str) {
    assert!(!is_explicit_keepalive_disable(Some(&value(raw))));
}

#[test]
fn a_missing_value_does_not_disable() {
    assert!(!is_explicit_keepalive_disable(None));
}

#[rstest]
#[case::double_underscore(r#""0__0""#)]
#[case::leading_underscore(r#""_0""#)]
#[case::trailing_underscore(r#""0_""#)]
#[case::negative_double_underscore(r#""-1__0""#)]
#[case::negative_leading_underscore(r#""-_1""#)]
#[case::negative_trailing_underscore(r#""-1_""#)]
#[case::underscore_before_dot(r#""0_.0""#)]
#[case::underscore_after_dot(r#""0._0""#)]
#[case::underscore_around_exponent(r#""0e_0""#)]
#[case::space_inside(r#""1 0""#)]
#[case::word(r#""info""#)]
fn bad_number_text_does_not_disable(#[case] raw: &str) {
    assert!(!is_explicit_keepalive_disable(Some(&value(raw))));
}

#[rstest]
#[case::underscored_zero(r#""0_0""#, true)]
#[case::underscored_negative(r#""-1_0""#, true)]
#[case::decimal_with_underscore(r#""0_0.0""#, true)]
#[case::exponent_with_underscore(r#""1_0e-2""#, false)]
#[case::leading_dot(r#"".25""#, false)]
fn number_text_follows_the_float_grammar(#[case] raw: &str, #[case] disables: bool) {
    assert_eq!(is_explicit_keepalive_disable(Some(&value(raw))), disables);
}

#[test]
fn without_a_router_there_is_no_deployment_config() {
    assert_eq!(
        deployment_keepalive(None, &map(r#"{"model": "gpt"}"#), &JsonObject::new()),
        None
    );
}

#[test]
fn a_known_model_id_reads_its_keepalive_and_override() {
    let fake = fake_router(FakeRouter::default().deployment(
        "dep-1",
        r#"{"keepalive_seconds": 7, "allow_client_keepalive_override": true}"#,
    ));
    let router = dyn_router(Arc::clone(&fake));
    let config = deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": "gpt"}"#),
        &map(r#"{"model_id": "dep-1"}"#),
    );
    assert_eq!(
        config,
        Some(DeploymentKeepalive {
            seconds: Some(JsonValue::from(7_i64)),
            allow_client_override: true,
        })
    );
    assert_eq!(fake.deployment_calls(), 1);
    assert_eq!(fake.group_calls(), 0);
}

#[test]
fn a_null_deployment_keepalive_has_no_value() {
    let fake =
        fake_router(FakeRouter::default().deployment("dep-1", r#"{"keepalive_seconds": null}"#));
    let router = dyn_router(Arc::clone(&fake));
    let config = deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": "gpt"}"#),
        &map(r#"{"model_id": "dep-1"}"#),
    );
    assert_eq!(
        config,
        Some(DeploymentKeepalive {
            seconds: None,
            allow_client_override: false,
        })
    );
}

#[test]
fn a_stale_model_id_never_falls_back_to_the_group() {
    let fake = fake_router(
        FakeRouter::default().group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#),
    );
    let router = dyn_router(Arc::clone(&fake));
    let config = deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": "gpt"}"#),
        &map(r#"{"model_id": "removed"}"#),
    );
    assert_eq!(config, None);
    assert_eq!(fake.group_calls(), 0);
}

#[test]
fn a_non_string_model_asks_for_the_group_without_a_name() {
    let fake = fake_router(
        FakeRouter::default().group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#),
    );
    let router = dyn_router(Arc::clone(&fake));
    let config = deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": 5}"#),
        &JsonObject::new(),
    );
    assert_eq!(config, None);
    assert_eq!(fake.requested_groups(), vec![None]);
}

#[test]
fn an_empty_model_id_falls_back_to_the_group() {
    let fake = fake_router(
        FakeRouter::default().group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#),
    );
    let router = dyn_router(Arc::clone(&fake));
    let config = deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": "gpt"}"#),
        &map(r#"{"model_id": ""}"#),
    );
    assert_eq!(
        config,
        Some(DeploymentKeepalive {
            seconds: Some(JsonValue::from(5_i64)),
            allow_client_override: false,
        })
    );
    assert_eq!(fake.deployment_calls(), 0);
    assert_eq!(fake.group_calls(), 1);
}

#[rstest]
#[case::bool_true(r#"{"allow_client_keepalive_override": true}"#, true)]
#[case::bool_false(r#"{"allow_client_keepalive_override": false}"#, false)]
#[case::missing("{}", false)]
#[case::text_true(r#"{"allow_client_keepalive_override": "true"}"#, false)]
#[case::number_one(r#"{"allow_client_keepalive_override": 1}"#, false)]
#[case::null(r#"{"allow_client_keepalive_override": null}"#, false)]
fn the_client_override_flag_is_the_json_bool(#[case] params: &str, #[case] expected: bool) {
    let fake = fake_router(FakeRouter::default().deployment("dep-1", params));
    let router = dyn_router(Arc::clone(&fake));
    let config = deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": "gpt"}"#),
        &map(r#"{"model_id": "dep-1"}"#),
    )
    .expect("known deployment");
    assert_eq!(config.allow_client_override, expected);
}

fn group_config(deployments: &str) -> Option<DeploymentKeepalive> {
    let fake = fake_router(FakeRouter::default().group("gpt", deployments));
    let router = dyn_router(Arc::clone(&fake));
    deployment_keepalive(
        Some(router.as_ref()),
        &map(r#"{"model": "gpt"}"#),
        &JsonObject::new(),
    )
}

#[rstest]
#[case::one_deployment(r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#, Some(5))]
#[case::number_and_float_agree(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 5.0}}]"#,
    Some(5)
)]
#[case::number_and_text_differ(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": "5"}}]"#,
    None
)]
#[case::two_numbers_differ(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 6}}]"#,
    None
)]
#[case::one_deployment_leaves_it_unset(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {}}]"#,
    None
)]
#[case::one_deployment_has_null(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": null}}]"#,
    None
)]
#[case::override_flags_differ(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 5, "allow_client_keepalive_override": true}}]"#,
    None
)]
#[case::no_deployments("[]", None)]
fn group_deployments_agree_only_on_one_pair(#[case] list: &str, #[case] expected: Option<i64>) {
    let expected_config = expected.map(|seconds| DeploymentKeepalive {
        seconds: Some(JsonValue::from(seconds)),
        allow_client_override: false,
    });
    assert_eq!(group_config(list), expected_config);
}

#[rstest]
#[case::neither_has_params(r#"[{}, {"litellm_params": {}}]"#)]
#[case::null_matches_a_missing_key(
    r#"[{"litellm_params": {"keepalive_seconds": null}}, {"litellm_params": {}}]"#
)]
#[case::params_is_not_an_object(r#"[{"litellm_params": "text"}, {}]"#)]
fn deployments_without_params_agree_on_no_value(#[case] list: &str) {
    assert_eq!(
        group_config(list),
        Some(DeploymentKeepalive {
            seconds: None,
            allow_client_override: false,
        })
    );
}

#[test]
fn a_deployment_hard_disable_beats_an_allowed_client_value() {
    let fake = fake_router(FakeRouter::default().deployment(
        "dep-1",
        r#"{"keepalive_seconds": 0, "allow_client_keepalive_override": true}"#,
    ));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt", "keepalive_seconds": 5}"#,
        r#"{"model_id": "dep-1"}"#,
        None,
    );
    assert_seconds(resolved, 0.0);
}

#[rstest]
#[case::allowed(true, r#"{"model": "gpt", "keepalive_seconds": 10}"#, 10.0_f64)]
#[case::not_allowed(false, r#"{"model": "gpt", "keepalive_seconds": 10}"#, 5.0_f64)]
#[case::allowed_with_null_client(true, r#"{"model": "gpt", "keepalive_seconds": null}"#, 5.0_f64)]
fn the_client_value_applies_only_when_allowed(
    #[case] allow: bool,
    #[case] request: &str,
    #[case] expected: f64,
) {
    let params =
        format!(r#"{{"keepalive_seconds": 5, "allow_client_keepalive_override": {allow}}}"#);
    let fake = fake_router(FakeRouter::default().deployment("dep-1", &params));
    let resolved = resolve(&fake, request, r#"{"model_id": "dep-1"}"#, None);
    assert_seconds(resolved, expected);
}

#[test]
fn the_deployment_value_beats_the_global_one() {
    let fake =
        fake_router(FakeRouter::default().deployment("dep-1", r#"{"keepalive_seconds": 5}"#));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt"}"#,
        r#"{"model_id": "dep-1"}"#,
        Some(PingSetting::Seconds(7.0)),
    );
    assert_seconds(resolved, 5.0);
}

#[test]
fn the_global_setting_applies_when_the_group_disagrees() {
    let fake = fake_router(FakeRouter::default().group(
        "gpt",
        r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 6}}]"#,
    ));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt", "keepalive_seconds": 9}"#,
        "{}",
        Some(PingSetting::Seconds(7.0)),
    );
    assert_seconds(resolved, 7.0);
}

#[rstest]
#[case::text_global(Some(PingSetting::Text("7".to_owned())), 7.0_f64)]
#[case::unparsable_global(Some(PingSetting::Text("later".to_owned())), 0.0_f64)]
#[case::zero_global(Some(PingSetting::Seconds(0.0)), 0.0_f64)]
#[case::negative_global(Some(PingSetting::Seconds(-3.0)), 0.0_f64)]
#[case::no_global(None, 0.0_f64)]
fn the_global_setting_is_the_last_fallback(
    #[case] global: Option<PingSetting>,
    #[case] expected: f64,
) {
    assert_seconds(
        resolve_without_router(r#"{"model": "gpt"}"#, "{}", global),
        expected,
    );
}

#[rstest]
#[case::zero_deployment(r#"{"keepalive_seconds": 0}"#)]
#[case::negative_text(r#"{"keepalive_seconds": "-1"}"#)]
#[case::unparsable_text(r#"{"keepalive_seconds": "fast"}"#)]
#[case::bool_value(r#"{"keepalive_seconds": true}"#)]
#[case::object_value(r#"{"keepalive_seconds": {}}"#)]
fn a_deployment_value_that_is_off_resolves_to_zero(#[case] params: &str) {
    let fake = fake_router(FakeRouter::default().deployment("dep-1", params));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt"}"#,
        r#"{"model_id": "dep-1"}"#,
        Some(PingSetting::Seconds(7.0)),
    );
    assert_seconds(resolved, 0.0);
}

#[test]
fn an_unparsable_client_value_does_not_fall_back() {
    let fake = fake_router(FakeRouter::default().deployment(
        "dep-1",
        r#"{"keepalive_seconds": 5, "allow_client_keepalive_override": true}"#,
    ));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt", "keepalive_seconds": "fast"}"#,
        r#"{"model_id": "dep-1"}"#,
        Some(PingSetting::Seconds(7.0)),
    );
    assert_seconds(resolved, 0.0);
}

#[rstest]
#[case::below_min(r#"{"keepalive_seconds": 0.5}"#, 1.0_f64)]
#[case::above_max(r#"{"keepalive_seconds": 1000}"#, 300.0_f64)]
#[case::infinity_text(r#"{"keepalive_seconds": "inf"}"#, 300.0_f64)]
#[case::nan_text(r#"{"keepalive_seconds": "nan"}"#, 1.0_f64)]
#[case::at_min(r#"{"keepalive_seconds": 1}"#, 1.0_f64)]
#[case::at_max(r#"{"keepalive_seconds": 300}"#, 300.0_f64)]
#[case::inside(r#"{"keepalive_seconds": 2.5}"#, 2.5_f64)]
fn deployment_values_clamp_into_the_allowed_range(#[case] params: &str, #[case] expected: f64) {
    let fake = fake_router(FakeRouter::default().deployment("dep-1", params));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt"}"#,
        r#"{"model_id": "dep-1"}"#,
        None,
    );
    assert_seconds(resolved, expected);
}

#[rstest]
#[case::infinity(f64::INFINITY, 300.0_f64)]
#[case::nan(f64::NAN, 1.0_f64)]
#[case::inside(7.5_f64, 7.5_f64)]
fn global_seconds_clamp_into_the_allowed_range(#[case] seconds: f64, #[case] expected: f64) {
    assert_seconds(
        resolve_without_router(
            r#"{"model": "gpt"}"#,
            "{}",
            Some(PingSetting::Seconds(seconds)),
        ),
        expected,
    );
}

#[test]
fn a_clamped_interval_is_logged() {
    let log = NamedTempFile::new().expect("log file");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("log handle")))
        .finish();
    let guard = set_default(subscriber);
    let fake =
        fake_router(FakeRouter::default().deployment("dep-1", r#"{"keepalive_seconds": 0.5}"#));
    let resolved = resolve(
        &fake,
        r#"{"model": "gpt"}"#,
        r#"{"model_id": "dep-1"}"#,
        None,
    );
    drop(guard);
    assert_seconds(resolved, 1.0);
    let output = fs::read_to_string(log.path()).expect("log output");
    assert!(
        output.contains("keepalive_seconds=0.5 clamped to 1 [min=1.0, max=300.0]"),
        "{output}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_resolver_serves_a_repeated_model_id_from_its_memo() {
    let fake =
        fake_router(FakeRouter::default().deployment("dep-1", r#"{"keepalive_seconds": 5}"#));
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let hidden = map(r#"{"model_id": "dep-1"}"#);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    advance(Duration::from_secs(4)).await;
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_eq!(fake.deployment_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_resolver_resolves_again_after_the_ttl() {
    let fake =
        fake_router(FakeRouter::default().deployment("dep-1", r#"{"keepalive_seconds": 5}"#));
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let hidden = map(r#"{"model_id": "dep-1"}"#);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    advance(Duration::from_secs(5)).await;
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_eq!(fake.deployment_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_resolver_resolves_another_model_id() {
    let fake = fake_router(
        FakeRouter::default()
            .deployment("dep-1", r#"{"keepalive_seconds": 5}"#)
            .deployment("dep-2", r#"{"keepalive_seconds": 6}"#),
    );
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    assert_seconds(resolver.resolve(&map(r#"{"model_id": "dep-1"}"#)), 5.0);
    assert_seconds(resolver.resolve(&map(r#"{"model_id": "dep-2"}"#)), 6.0);
    assert_eq!(fake.deployment_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_resolver_always_resolves_a_chunk_without_a_model_id() {
    let fake = fake_router(
        FakeRouter::default().group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#),
    );
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    assert_seconds(resolver.resolve(&JsonObject::new()), 5.0);
    assert_seconds(resolver.resolve(&JsonObject::new()), 5.0);
    assert_eq!(fake.group_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_chunk_without_a_model_id_leaves_the_memo_alone() {
    let fake = fake_router(
        FakeRouter::default()
            .deployment("dep-1", r#"{"keepalive_seconds": 5}"#)
            .group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 7}}]"#),
    );
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let hidden = map(r#"{"model_id": "dep-1"}"#);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_seconds(resolver.resolve(&JsonObject::new()), 7.0);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_eq!(fake.deployment_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_chunk_without_a_model_id_does_not_restart_the_memo_ttl() {
    let fake = fake_router(
        FakeRouter::default()
            .deployment("dep-1", r#"{"keepalive_seconds": 5}"#)
            .group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 7}}]"#),
    );
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let hidden = map(r#"{"model_id": "dep-1"}"#);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    advance(Duration::from_secs(3)).await;
    assert_seconds(resolver.resolve(&JsonObject::new()), 7.0);
    advance(Duration::from_secs(3)).await;
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_eq!(fake.deployment_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_empty_model_id_never_uses_the_memo() {
    let fake = fake_router(
        FakeRouter::default().group("gpt", r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#),
    );
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let hidden = map(r#"{"model_id": ""}"#);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_eq!(fake.group_calls(), 2);
    assert_eq!(fake.deployment_calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_memo_ttl_counts_from_the_last_real_resolution() {
    let fake =
        fake_router(FakeRouter::default().deployment("dep-1", r#"{"keepalive_seconds": 5}"#));
    let mut resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let hidden = map(r#"{"model_id": "dep-1"}"#);
    assert_seconds(resolver.resolve(&hidden), 5.0);
    advance(Duration::from_secs(3)).await;
    assert_seconds(resolver.resolve(&hidden), 5.0);
    advance(Duration::from_secs(3)).await;
    assert_seconds(resolver.resolve(&hidden), 5.0);
    assert_eq!(fake.deployment_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_zero_interval_forwards_items_as_they_arrive() {
    let fake = fake_router(FakeRouter::default());
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::once(async {
        sleep(Duration::from_secs(5)).await;
        chunk()
    });
    let mut beats = Box::pin(with_keepalive(items, resolver, 0.0));
    let started = Instant::now();
    assert_eq!(beats.next().await, Some(Beat::Item(chunk())));
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert_eq!(beats.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn a_quiet_stream_yields_a_ping_each_interval() {
    let fake = fake_router(FakeRouter::default());
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let mut beats = Box::pin(with_keepalive(
        stream::pending::<FakeChunk>(),
        resolver,
        1.0,
    ));
    let started = Instant::now();
    assert_eq!(beats.next().await, Some(Beat::Ping));
    assert_eq!(beats.next().await, Some(Beat::Ping));
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn an_item_that_arrives_after_a_ping_is_not_lost() {
    let fake = fake_router(FakeRouter::default());
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::once(async {
        sleep(Duration::from_secs(2)).await;
        chunk()
    });
    let mut beats = Box::pin(with_keepalive(items, resolver, 1.0));
    let started = Instant::now();
    assert_eq!(beats.next().await, Some(Beat::Ping));
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    assert_eq!(beats.next().await, Some(Beat::Item(chunk())));
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!(beats.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn the_interval_after_an_item_comes_from_that_item() {
    let fake = fake_router(FakeRouter::default().deployment("slow", r#"{"keepalive_seconds": 4}"#));
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::iter(vec![hidden_chunk("slow")]).chain(stream::pending());
    let mut beats = Box::pin(with_keepalive(items, resolver, 1.0));
    let started = Instant::now();
    assert_eq!(beats.next().await, Some(Beat::Item(hidden_chunk("slow"))));
    assert_eq!(beats.next().await, Some(Beat::Ping));
    assert_eq!(started.elapsed(), Duration::from_secs(4));
}

#[tokio::test(start_paused = true)]
async fn a_deployment_can_turn_heartbeats_on_mid_stream() {
    let fake = fake_router(
        FakeRouter::default()
            .deployment("off", r#"{"keepalive_seconds": 0}"#)
            .deployment("on", r#"{"keepalive_seconds": 2}"#),
    );
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items =
        stream::iter(vec![hidden_chunk("off"), hidden_chunk("on")]).chain(stream::pending());
    let mut beats = Box::pin(with_keepalive(items, resolver, 5.0));
    let started = Instant::now();
    assert_eq!(beats.next().await, Some(Beat::Item(hidden_chunk("off"))));
    assert_eq!(beats.next().await, Some(Beat::Item(hidden_chunk("on"))));
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(beats.next().await, Some(Beat::Ping));
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn a_deployment_can_turn_heartbeats_off_mid_stream() {
    let fake = fake_router(FakeRouter::default().deployment("off", r#"{"keepalive_seconds": 0}"#));
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::iter(vec![hidden_chunk("off")]).chain(stream::pending());
    let mut beats = Box::pin(with_keepalive(items, resolver, 5.0));
    assert_eq!(beats.next().await, Some(Beat::Item(hidden_chunk("off"))));
    let quiet = timeout(Duration::from_secs(10), beats.next()).await;
    quiet.expect_err("a disabled interval must stay quiet");
}

#[tokio::test(start_paused = true)]
async fn the_wrapper_ends_when_the_items_end() {
    let fake = fake_router(FakeRouter::default());
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::iter(vec![chunk()]);
    let mut beats = Box::pin(with_keepalive(items, resolver, 1.0));
    assert_eq!(beats.next().await, Some(Beat::Item(chunk())));
    assert_eq!(beats.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn the_wrapper_ends_the_wait_when_the_items_end() {
    let fake = fake_router(FakeRouter::default());
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::empty::<FakeChunk>();
    let mut beats = Box::pin(with_keepalive(items, resolver, 1.0));
    assert_eq!(beats.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn the_wrapper_ends_after_an_item_with_a_positive_interval() {
    let fake = fake_router(FakeRouter::default().deployment("slow", r#"{"keepalive_seconds": 4}"#));
    let resolver = resolver_for(&fake, r#"{"model": "gpt"}"#, None);
    let items = stream::iter(vec![hidden_chunk("slow")]);
    let mut beats = Box::pin(with_keepalive(items, resolver, 1.0));
    assert_eq!(beats.next().await, Some(Beat::Item(hidden_chunk("slow"))));
    assert_eq!(beats.next().await, None);
}
