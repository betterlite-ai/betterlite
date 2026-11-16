use std::collections::BTreeSet;

use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{data, info, moment};
use crate::budget_permission_rows::JsonData;
use crate::ptu::charges::deployment::{
    RouterEntry, StoredModel, config_deployments, parse_ptu_model, scan_deployments,
};

const TERMS: &str = r#"{"ptu_count": 2, "cost_per_ptu_per_hour": 0.5, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z"}"#;

fn entry(name: &str, model_info: &str) -> RouterEntry {
    RouterEntry {
        model_name: name.to_owned(),
        model_info: data(model_info),
    }
}

fn stored(id: &str, name: &str, model_info: Option<JsonData>) -> StoredModel {
    StoredModel {
        model_id: id.to_owned(),
        model_name: name.to_owned(),
        model_info,
    }
}

#[test]
fn valid_terms_turn_a_stored_row_into_a_model() {
    let row = stored("model-a", "display", Some(data(TERMS)));
    let parsed = scan_deployments(&[row], &[]).models;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].model_id, "model-a");
    assert_eq!(parsed[0].model_name, "display");
    assert_eq!(parsed[0].team_id, "team-1");
    assert_eq!(parsed[0].ptu_count, 2);
    assert_eq!(parsed[0].cost_per_ptu_per_hour, Decimal::new(5, 1));
    assert_eq!(parsed[0].effective_from, moment("2026-01-01T00:00:00Z"));
    assert_eq!(parsed[0].effective_to, None);
}

#[rstest]
#[case::no_terms(r#"{"other": 1}"#)]
#[case::missing_team(r#"{"ptu_count": 2, "cost_per_ptu_per_hour": 0.5, "ptu_effective_from": "2026-01-01T00:00:00Z"}"#)]
#[case::missing_start(r#"{"ptu_count": 2, "cost_per_ptu_per_hour": 0.5, "team_id": "team-1"}"#)]
#[case::zero_count(r#"{"ptu_count": 0, "cost_per_ptu_per_hour": 0.5, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z"}"#)]
#[case::inverted_window(r#"{"ptu_count": 2, "cost_per_ptu_per_hour": 0.5, "team_id": "team-1", "ptu_effective_from": "2026-01-02T00:00:00Z", "ptu_effective_to": "2026-01-01T00:00:00Z"}"#)]
fn invalid_terms_leave_the_row_unpriced(#[case] model_info: &str) {
    let mapping = info(model_info);
    assert!(parse_ptu_model("model-a", "display", &mapping).is_none());
}

#[rstest]
#[case::declared(r#""team-public""#, "team-public")]
#[case::empty_declared(r#""""#, "display")]
#[case::non_string_declared("12", "display")]
fn public_name_rules(#[case] declared: &str, #[case] expected: &str) {
    let mapping = info(&format!(
        r#"{{"ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z", "team_public_model_name": {declared}}}"#
    ));
    let parsed = parse_ptu_model("model-a", "display", &mapping).unwrap();
    assert_eq!(parsed.model_name, expected);
}

#[test]
fn missing_public_name_uses_the_row_name() {
    let mapping = info(TERMS);
    let parsed = parse_ptu_model("model-a", "display", &mapping).unwrap();
    assert_eq!(parsed.model_name, "display");
}

#[test]
fn an_empty_model_id_survives_parsing() {
    let mapping = info(TERMS);
    let parsed = parse_ptu_model("", "display", &mapping).unwrap();
    assert_eq!(parsed.model_id, "");
}

#[rstest]
#[case::missing(r#"{"model_name": "n"}"#)]
#[case::empty(r#"{"id": "", "model_name": "n"}"#)]
#[case::number(r#"{"id": 7, "model_name": "n"}"#)]
#[case::null(r#"{"id": null, "model_name": "n"}"#)]
#[case::nonobject(r#"{"id": {"value": "x"}}"#)]
fn router_entries_need_a_nonempty_string_id(#[case] model_info: &str) {
    assert!(config_deployments(&[entry("n", model_info)], &BTreeSet::new()).is_empty());
}

#[test]
fn router_entries_accept_a_json_text_model_info() {
    let router_entry = RouterEntry {
        model_name: "n".to_owned(),
        model_info: JsonData::String(r#"{"id": "model-a", "ptu_count": 2}"#.to_owned()),
    };
    let deployments = config_deployments(&[router_entry], &BTreeSet::new());
    let deployment = deployments.first().unwrap();
    assert_eq!(deployment.model_id, "model-a");
    assert_eq!(
        deployment.model_info.get("ptu_count"),
        Some(&JsonData::Number(serde_json::Number::from(2_i32)))
    );
}

#[test]
fn a_router_snapshot_does_not_follow_later_edits() {
    let mut router_entry = entry("n", r#"{"id": "model-a", "ptu_count": 2}"#);
    let deployments = config_deployments(std::slice::from_ref(&router_entry), &BTreeSet::new());
    let deployment = deployments.first().unwrap();
    router_entry.model_info = data(r#"{"id": "model-a", "ptu_count": 9}"#);
    assert_eq!(
        deployment.model_info.get("ptu_count"),
        Some(&JsonData::Number(serde_json::Number::from(2_i32)))
    );
}

#[rstest]
#[case::db_model_true(r#"{"id": "cfg-a", "db_model": true}"#, false)]
#[case::db_model_false(r#"{"id": "cfg-a", "db_model": false}"#, true)]
#[case::db_model_text(r#"{"id": "cfg-a", "db_model": "true"}"#, true)]
#[case::db_model_number(r#"{"id": "cfg-a", "db_model": 1}"#, true)]
#[case::original_model_id(r#"{"id": "cfg-a", "original_model_id": "db-a"}"#, false)]
#[case::original_model_id_null(r#"{"id": "cfg-a", "original_model_id": null}"#, true)]
#[case::plain_config(r#"{"id": "cfg-a"}"#, true)]
fn config_deployments_skip_credential_copies(#[case] model_info: &str, #[case] kept: bool) {
    let owned = BTreeSet::new();
    let kept_entries = config_deployments(&[entry("n", model_info)], &owned);
    assert_eq!(kept_entries.len(), usize::from(kept));
}

#[test]
fn a_db_owned_id_is_not_a_config_deployment() {
    let owned = BTreeSet::from(["cfg-a".to_owned()]);
    assert!(config_deployments(&[entry("n", r#"{"id": "cfg-a"}"#)], &owned).is_empty());
}

#[test]
fn config_deployments_keep_router_order() {
    let entries = [
        entry("n", r#"{"id": "cfg-b"}"#),
        entry("n", r#"{"id": "cfg-a"}"#),
    ];
    let kept = config_deployments(&entries, &BTreeSet::new());
    assert_eq!(kept[0].model_id, "cfg-b");
    assert_eq!(kept[1].model_id, "cfg-a");
}

#[test]
fn the_scan_reads_rows_before_config_and_keeps_every_id() {
    let rows = [
        stored("db-a", "stored", Some(data(TERMS))),
        stored("db-b", "invalid", Some(data(r#"{"other": 1}"#))),
        stored("", "anonymous", Some(data(TERMS))),
        stored("db-c", "no info", None),
    ];
    let entries = [
        entry(
            "n",
            r#"{"id": "cfg-a", "ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z"}"#,
        ),
        entry(
            "n",
            r#"{"id": "db-a", "ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z"}"#,
        ),
    ];
    let loaded = scan_deployments(&rows, &entries);
    let ids: Vec<&str> = loaded
        .models
        .iter()
        .map(|model| model.model_id.as_str())
        .collect();
    assert_eq!(ids, ["db-a", "", "cfg-a"]);
    assert_eq!(
        loaded.scanned_ids,
        BTreeSet::from([
            "".to_owned(),
            "cfg-a".to_owned(),
            "db-a".to_owned(),
            "db-b".to_owned(),
            "db-c".to_owned(),
        ])
    );
}

#[test]
fn an_unpriced_empty_model_id_is_not_scanned() {
    let rows = [stored("", "invalid", Some(data(r#"{"other": 1}"#)))];
    let loaded = scan_deployments(&rows, &[]);
    assert!(loaded.models.is_empty());
    assert!(loaded.scanned_ids.is_empty());
}

fn published(id: &str) -> String {
    format!(
        r#"{{"id": "{id}", "ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "team-1", "ptu_effective_from": "2026-01-01T00:00:00Z", "team_public_model_name": "shared"}}"#
    )
}

#[test]
fn a_shared_public_name_keeps_each_deployment_id() {
    let first = published("cfg-a");
    let second = published("cfg-b");
    let entries = [entry("n1", &first), entry("n2", &second)];
    let loaded = scan_deployments(&[], &entries);
    assert_eq!(loaded.models.len(), 2);
    assert_eq!(loaded.models[0].model_name, "shared");
    assert_eq!(loaded.models[1].model_name, "shared");
    assert_ne!(loaded.models[0].model_id, loaded.models[1].model_id);
}
