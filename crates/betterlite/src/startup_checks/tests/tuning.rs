use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::ports::PortError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::startup_checks::tuning::{
    Deployment, TuningBaselines, heuristic_v1_router_fingerprint, is_mutable_tuned_candidate,
    mutable_tuned_identities, router_identity, snapshot_tuning_baselines, tuning_fingerprint,
    tuning_limit_violation, uses_heuristic_v1,
};
use crate::startup_checks::tuning_baseline::{
    AUTO_ROUTER_LICENSE_REMEDY, BaselineWriteError, TUNING_BASELINE_PARAM_NAME,
    TuningBaselineStore, enforce_tuning_baseline, load_tuning_baselines,
};

use super::{EMPTY_FINGERPRINT, capture_logs, deployment, json};

#[test]
fn the_empty_configuration_fingerprint_is_pinned() {
    assert_eq!(tuning_fingerprint(None).as_deref(), Some(EMPTY_FINGERPRINT));
    assert_eq!(
        tuning_fingerprint(Some(&Json::Object(IndexMap::new()))).as_deref(),
        Some(EMPTY_FINGERPRINT)
    );
    assert_eq!(
        tuning_fingerprint(Some(&Json::Null)).as_deref(),
        Some(EMPTY_FINGERPRINT)
    );
}

#[test]
fn fingerprint_normalizes_supplied_fields() {
    let config = json(r#"{"code_keywords": ["a"]}"#);
    assert_eq!(
        tuning_fingerprint(Some(&config)).as_deref(),
        Some("288bc4eb844e342af70ed64a73035eb69f5500753e8d09c3013d59ce6f6b123d")
    );
    let integer_boundary = json(r#"{"tier_boundaries": {"simple_medium": 1}}"#);
    assert_eq!(
        tuning_fingerprint(Some(&integer_boundary)).as_deref(),
        Some("3b54ea829fb0b94a38cf893ba6131a4a524f8b6cd9d8045f04c9a3b0fb189306")
    );
    let fraction = json(r#"{"tier_boundaries": {"simple_medium": 0.15}}"#);
    assert_eq!(
        tuning_fingerprint(Some(&fraction)).as_deref(),
        Some("1c00f7b54a644277f0160b22e55b449f063fef5654803c142eacd97a58848d87")
    );
}

#[test]
fn fingerprint_keeps_a_supplied_null_for_an_optional_field() {
    let config = json(r#"{"code_keywords": null}"#);
    assert_eq!(
        tuning_fingerprint(Some(&config)).as_deref(),
        Some("2bd74cf21590031d9232f212764d179a52c064dd0fd3bdcf87b7984e7ea408d4")
    );
}

#[test]
fn fingerprint_rejects_a_null_for_a_required_field() {
    for text in [
        r#"{"tier_boundaries": null}"#,
        r#"{"token_thresholds": null}"#,
        r#"{"dimension_weights": null}"#,
        r#"{"custom_dimensions": null}"#,
    ] {
        assert_eq!(tuning_fingerprint(Some(&json(text))), None, "{text}");
    }
}

#[test]
fn fingerprint_omits_the_built_in_escalation_marker() {
    for text in [
        r#"{"escalation_keywords": ["LITELLM ESCALATE"]}"#,
        r#"{"escalation_keywords": ["  LITELLM ESCALATE "]}"#,
        r#"{"escalation_keywords": null}"#,
    ] {
        assert_eq!(
            tuning_fingerprint(Some(&json(text))).as_deref(),
            Some(EMPTY_FINGERPRINT),
            "{text}"
        );
    }
    let other = json(r#"{"escalation_keywords": ["other"]}"#);
    assert_eq!(
        tuning_fingerprint(Some(&other)).as_deref(),
        Some("64a1d6812c14712781f70d7c24d49136fb1752cf193bf03e39bfb7eb299b14bd")
    );
}

#[test]
fn fingerprint_dumps_dimensions_with_their_defaults() {
    let binary =
        json(r#"{"custom_dimensions": [{"name": "alpha", "weight": 1, "keywords": ["x"]}]}"#);
    assert_eq!(
        tuning_fingerprint(Some(&binary)).as_deref(),
        Some("d212de988a18977ace0f56562db87f20810366fbc690ca70647e357b5b063d9f")
    );
    let match_count = json(
        r#"{"custom_dimensions": [{"name": "alpha", "weight": 1, "patterns": ["a"], "scoring_mode": "match_count"}]}"#,
    );
    assert_eq!(
        tuning_fingerprint(Some(&match_count)).as_deref(),
        Some("b5e0dbd3645fed6f12a863d9867cc88ae98f32725dca5a8d78923651f981ec0e")
    );
}

#[test]
fn fingerprint_keeps_punctuation_and_spacing_inside_a_string() {
    let config = json(r#"{"code_keywords": ["a, b: c \" d \\ e"]}"#);
    assert_eq!(
        tuning_fingerprint(Some(&config)).as_deref(),
        Some("5505d0e544c7b659fd73c9344580db42ae136755a6b1ce9e6915846550d82151")
    );
}

#[test]
fn fingerprint_normalizes_keyword_tier_rules() {
    let rules = json(
        r#"{"keyword_tier_rules": [{"keywords": ["  hi  ", " ", "there"], "tier": "MEDIUM"}]}"#,
    );
    assert_eq!(
        tuning_fingerprint(Some(&rules)).as_deref(),
        Some("f52feeb594e4625ed67bc2b097d2db7675c08d35d333d8bae2f618898060fce0")
    );
}

#[test]
fn fingerprint_dumps_numbers_and_sorts_map_keys() {
    let cases = [
        (
            r#"{"reasoning_override_min_score": 0.5}"#,
            "d5c5d1e318adcbbe0223ff13defb7828c2a633f1299924ea634f6861dbc39095",
        ),
        (
            r#"{"token_thresholds": {"low": 10}}"#,
            "9c59e8ec6997b80083b9600772f439c08dc9945fa72acf9836fa1c19d4612303",
        ),
        (
            r#"{"dimension_weights": {"alpha": 0.5}}"#,
            "f963c7ff92e98bdb96aca17289199c2854024cf0320b77b261f96d27eccc15f5",
        ),
        (
            r#"{"tier_boundaries": {"b": 0.2, "a": 0.1}}"#,
            "9af5f7c359f1fc1b3abb244219898744aea984d49b8c44e31a48e2ad5fe34aa1",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(
            tuning_fingerprint(Some(&json(text))).as_deref(),
            Some(expected),
            "{text}"
        );
    }
}

#[test]
fn fingerprint_is_none_for_a_config_litellm_rejects() {
    let bad_name = json(r#"{"custom_dimensions": [{"name": "9bad", "weight": 0.5}]}"#);
    assert_eq!(tuning_fingerprint(Some(&bad_name)), None);
    let unknown_classifier = json(r#"{"classifier_type": "nope"}"#);
    assert_eq!(tuning_fingerprint(Some(&unknown_classifier)), None);
}

#[test]
fn uses_heuristic_v1_follows_classifier_type() {
    assert!(uses_heuristic_v1(None));
    for kind in ["heuristic", "heuristic_first", "hybrid"] {
        let config = json(&format!(r#"{{"classifier_type": "{kind}"}}"#));
        assert!(uses_heuristic_v1(Some(&config)), "{kind}");
    }
    for kind in ["llm", "capability", "heuristic_v2"] {
        let config = json(&format!(r#"{{"classifier_type": "{kind}"}}"#));
        assert!(!uses_heuristic_v1(Some(&config)), "{kind}");
    }
    assert!(!uses_heuristic_v1(Some(&Json::Null)));
    let numeric = json(r#"{"classifier_type": 5}"#);
    assert!(!uses_heuristic_v1(Some(&numeric)));
}

#[test]
fn router_identity_prefers_database_ids_and_sorts_tags() {
    let mut router = deployment("router", None);
    router.tags = vec!["b".to_owned(), "a".to_owned()];
    assert_eq!(
        router_identity(&router).as_deref(),
        Some(r#"yaml:["router",["a","b"]]"#)
    );
    router.db_model = true;
    router.model_info_id = Some("model-1".to_owned());
    assert_eq!(router_identity(&router).as_deref(), Some("db:model-1"));
    router.model_info_id = Some(String::new());
    assert_eq!(
        router_identity(&router).as_deref(),
        Some(r#"yaml:["router",["a","b"]]"#)
    );
    router.model_name = None;
    assert_eq!(router_identity(&router), None);
}

#[test]
fn from_entry_reads_the_deployment_fields() {
    let entry = json(
        r#"{"model_name": "router", "litellm_params": {"model": "auto_router/complexity_router/x", "tags": ["t"]}, "model_info": {"id": "m1", "db_model": true}}"#,
    );
    let router = Deployment::from_entry(&entry);
    assert_eq!(router.model_name.as_deref(), Some("router"));
    assert_eq!(
        router.model.as_deref(),
        Some("auto_router/complexity_router/x")
    );
    assert_eq!(router.tags, vec!["t".to_owned()]);
    assert_eq!(router.model_info_id.as_deref(), Some("m1"));
    assert!(router.db_model);
}

#[test]
fn heuristic_pairs_need_the_prefix_and_the_heuristic_classifier() {
    let router = deployment("router", None);
    let pair = heuristic_v1_router_fingerprint(&router).unwrap();
    assert_eq!(pair.0, r#"yaml:["router",[]]"#);
    assert_eq!(pair.1, EMPTY_FINGERPRINT);
    let mut other = deployment(
        "router",
        Some(json(
            r#"{"classifier_type": "llm", "classifier_llm_config": {"model": "gpt-4o"}}"#,
        )),
    );
    assert_eq!(heuristic_v1_router_fingerprint(&other), None);
    other.model = Some("gpt-4o".to_owned());
    assert_eq!(heuristic_v1_router_fingerprint(&other), None);
}

#[test]
fn mutable_candidates_compare_against_the_baseline_then_the_default() {
    let changed = deployment("changed", Some(json(r#"{"code_keywords": ["a"]}"#)));
    let untouched = deployment("untouched", None);
    assert!(is_mutable_tuned_candidate(
        &changed,
        &TuningBaselines::new()
    ));
    assert!(!is_mutable_tuned_candidate(
        &untouched,
        &TuningBaselines::new()
    ));
    let mut baselines = TuningBaselines::new();
    baselines.insert(
        r#"yaml:["changed",[]]"#.to_owned(),
        "288bc4eb844e342af70ed64a73035eb69f5500753e8d09c3013d59ce6f6b123d".to_owned(),
    );
    assert!(!is_mutable_tuned_candidate(&changed, &baselines));
    assert!(!is_mutable_tuned_candidate(&untouched, &baselines));
    let retuned = deployment("changed", Some(json(r#"{"code_keywords": ["b"]}"#)));
    assert!(is_mutable_tuned_candidate(&retuned, &baselines));
    let snapshot = snapshot_tuning_baselines(&[changed.clone(), untouched.clone()]);
    assert_eq!(snapshot.len(), 2);
    assert_eq!(
        snapshot[r#"yaml:["changed",[]]"#],
        "288bc4eb844e342af70ed64a73035eb69f5500753e8d09c3013d59ce6f6b123d"
    );
    assert_eq!(
        mutable_tuned_identities(&[changed, untouched], &TuningBaselines::new()).len(),
        1
    );
}

#[test]
fn tuning_limit_violation_starts_above_the_limit() {
    assert_eq!(tuning_limit_violation(0, None), None);
    assert_eq!(tuning_limit_violation(5, None), None);
    assert_eq!(tuning_limit_violation(1, Some(1)), None);
    assert_eq!(tuning_limit_violation(0, Some(0)), None);
    let violation = tuning_limit_violation(2, Some(1)).unwrap();
    assert_eq!(
        violation,
        "At most 1 auto-router(s) with changed heuristic scoring rules can be modified without an auto-router license. Keep this router on its recorded settings, or revert the other changed router to its baseline, or remove one of them. Selecting models does not use this allowance."
    );
}

// tuning baseline store

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RecordOutcome {
    #[default]
    Store,
    Unique,
    Fail,
}

#[derive(Default)]
struct BaselineStore {
    row: Mutex<Option<Json>>,
    winner: Mutex<Option<Json>>,
    outcome: RecordOutcome,
    find_error: Mutex<Option<String>>,
    recorded: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl TuningBaselineStore for BaselineStore {
    async fn find_baseline(&self, _param_name: &str) -> Result<Option<Json>, PortError> {
        if let Some(message) = self.find_error.lock().unwrap().as_deref() {
            return Err(PortError::new(message));
        }
        Ok(self.row.lock().unwrap().clone())
    }

    async fn record_baseline(
        &self,
        param_name: &str,
        param_value: &str,
    ) -> Result<(), BaselineWriteError> {
        self.recorded
            .lock()
            .unwrap()
            .push((param_name.to_owned(), param_value.to_owned()));
        match self.outcome {
            RecordOutcome::Store => {
                *self.row.lock().unwrap() = Some(Json::Text(param_value.to_owned()));
                Ok(())
            }
            RecordOutcome::Unique => {
                *self.row.lock().unwrap() = self.winner.lock().unwrap().take();
                Err(BaselineWriteError::UniqueViolation)
            }
            RecordOutcome::Fail => Err(BaselineWriteError::Database(PortError::new(
                "insert failed",
            ))),
        }
    }
}

fn baseline_store(row: Option<Json>) -> BaselineStore {
    BaselineStore {
        row: Mutex::new(row),
        ..BaselineStore::default()
    }
}

#[tokio::test]
async fn load_reads_a_json_string_row_and_an_object_row() {
    let text_row = baseline_store(Some(Json::Text(
        r#"{"yaml:abc": "ffff", "yaml:def": "eeee"}"#.to_owned(),
    )));
    let baselines = load_tuning_baselines(&text_row, &[]).await.unwrap();
    assert_eq!(baselines["yaml:abc"], "ffff");
    assert_eq!(baselines["yaml:def"], "eeee");

    let mut object = IndexMap::new();
    object.insert("yaml:abc".to_owned(), Json::Text("ffff".to_owned()));
    let object_row = baseline_store(Some(Json::Object(object)));
    let object_baselines = load_tuning_baselines(&object_row, &[]).await.unwrap();
    assert_eq!(object_baselines["yaml:abc"], "ffff");

    let list_row = baseline_store(Some(Json::Text("[1, 2]".to_owned())));
    assert!(
        load_tuning_baselines(&list_row, &[])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn load_records_the_first_snapshot() {
    let router = deployment("router", None);
    let store = baseline_store(None);
    let (logs, _guard) = capture_logs();
    let baselines = load_tuning_baselines(&store, &[router]).await.unwrap();
    assert_eq!(baselines.len(), 1);
    let recorded = store.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, TUNING_BASELINE_PARAM_NAME);
    let expected = format!(r#"{{"yaml:[\"router\",[]]": "{EMPTY_FINGERPRINT}"}}"#);
    assert_eq!(recorded[0].1, expected);
    assert!(
        logs.contents()
            .contains("Recorded heuristic-v1 tuning baseline for 1 auto-router(s)")
    );
}

#[tokio::test]
async fn load_adopts_the_competing_pods_row_on_a_unique_violation() {
    let router = deployment("router", None);
    let mut winner = IndexMap::new();
    winner.insert("yaml:other".to_owned(), Json::Text("abc".to_owned()));
    let store = BaselineStore {
        winner: Mutex::new(Some(Json::Object(winner))),
        outcome: RecordOutcome::Unique,
        ..BaselineStore::default()
    };
    let baselines = load_tuning_baselines(&store, &[router]).await.unwrap();
    assert_eq!(baselines.len(), 1);
    assert_eq!(baselines["yaml:other"], "abc");
}

#[tokio::test]
async fn load_skips_the_gate_when_the_row_cannot_be_read() {
    let mut find_failure = baseline_store(None);
    *find_failure.find_error.lock().unwrap() = Some("read failed".to_owned());
    let (logs, _guard) = capture_logs();
    assert_eq!(load_tuning_baselines(&find_failure, &[]).await, None);
    assert!(logs.contents().contains(
        "Heuristic-v1 tuning baseline unavailable, gate not enforced this boot: read failed"
    ));
}

#[tokio::test]
async fn load_skips_the_gate_when_the_snapshot_cannot_be_written() {
    let write_failure = BaselineStore {
        outcome: RecordOutcome::Fail,
        ..BaselineStore::default()
    };
    let (logs, _guard) = capture_logs();
    assert_eq!(load_tuning_baselines(&write_failure, &[]).await, None);
    assert!(logs.contents().contains("insert failed"));
}

#[tokio::test]
async fn load_skips_the_gate_when_the_recorded_row_is_not_json() {
    let decode_failure = baseline_store(Some(Json::Text("not json".to_owned())));
    let (logs, _guard) = capture_logs();
    assert_eq!(load_tuning_baselines(&decode_failure, &[]).await, None);
    assert!(
        logs.contents()
            .contains("the recorded heuristic-v1 tuning baseline row is not JSON")
    );
}

#[tokio::test]
async fn enforce_returns_baselines_within_the_limit_and_fails_above_it() {
    let store = baseline_store(Some(Json::Object(IndexMap::new())));
    let changed = vec![
        deployment("one", Some(json(r#"{"code_keywords": ["a"]}"#))),
        deployment("two", Some(json(r#"{"code_keywords": ["b"]}"#))),
    ];
    let within_limit = enforce_tuning_baseline(&store, &changed, Some(&[]), Some(2))
        .await
        .unwrap();
    assert!(within_limit.unwrap().is_empty());
    let error = enforce_tuning_baseline(&store, &changed, Some(&[]), Some(1))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "model_list: {} {AUTO_ROUTER_LICENSE_REMEDY}",
            tuning_limit_violation(2, Some(1)).unwrap()
        )
    );
}

#[tokio::test]
async fn enforce_warns_and_skips_when_the_database_models_cannot_be_read() {
    let store = baseline_store(None);
    let (logs, _guard) = capture_logs();
    let outcome = enforce_tuning_baseline(&store, &[], None, Some(0))
        .await
        .unwrap();
    assert!(outcome.is_none());
    assert!(
        logs.contents()
            .contains("Heuristic-v1 tuning baseline unavailable, gate not enforced this boot")
    );
    assert!(!logs.contents().contains("boot:"));
}

#[tokio::test]
async fn enforce_combines_config_and_database_deployments() {
    let store = baseline_store(Some(Json::Object(IndexMap::new())));
    let config = vec![deployment("one", Some(json(r#"{"code_keywords": ["a"]}"#)))];
    let database = vec![deployment("two", Some(json(r#"{"code_keywords": ["b"]}"#)))];
    let error = enforce_tuning_baseline(&store, &config, Some(&database), Some(1))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "model_list: {} {AUTO_ROUTER_LICENSE_REMEDY}",
            tuning_limit_violation(2, Some(1)).unwrap()
        )
    );
    assert!(
        enforce_tuning_baseline(&store, &config, Some(&database), Some(2))
            .await
            .unwrap()
            .is_some()
    );
}
