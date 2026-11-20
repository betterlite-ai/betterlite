#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;
use serde_json::json;

use super::super::model_guardrails::{
    Deployment, DeploymentGuardrails, merge_guardrails_with_existing, merge_model_level_guardrails,
};
use super::{json_object, object};

struct FakeDeployments {
    by_id: BTreeMap<String, Deployment>,
    by_name: BTreeMap<(String, Option<String>), Vec<Deployment>>,
}

impl DeploymentGuardrails for FakeDeployments {
    fn get_deployment(&self, model_id: &str) -> Option<Deployment> {
        self.by_id.get(model_id).cloned()
    }

    fn get_model_list(&self, model_name: &str, team_id: Option<&str>) -> Vec<Deployment> {
        let key = (model_name.to_owned(), team_id.map(str::to_owned));
        self.by_name.get(&key).cloned().unwrap_or_default()
    }
}

fn deployment(guardrails: JsonValue) -> Deployment {
    Deployment {
        guardrails: Some(guardrails),
    }
}

fn deployments_by_id(model_id: &str, guardrails: JsonValue) -> BTreeMap<String, Deployment> {
    BTreeMap::from([(model_id.to_owned(), deployment(guardrails))])
}

fn deployments_by_name(
    model_name: &str,
    team_id: Option<&str>,
    guardrails: Vec<JsonValue>,
) -> BTreeMap<(String, Option<String>), Vec<Deployment>> {
    let entries = guardrails.into_iter().map(deployment).collect();
    BTreeMap::from([((model_name.to_owned(), team_id.map(str::to_owned)), entries)])
}

fn model_info(id: JsonValue) -> JsonValue {
    json_object(vec![("model_info", json_object(vec![("id", id)]))])
}

fn guarded_metadata(id: &str, guardrails: JsonValue) -> JsonValue {
    json_object(vec![
        ("model_info", json_object(vec![("id", json!(id))])),
        ("guardrails", guardrails),
    ])
}

#[test]
fn model_guardrails_without_a_router_return_the_data_unchanged() {
    let request = object(vec![("metadata", json!("broken"))]);
    let merged = merge_model_level_guardrails(&request, None, true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_from_a_trusted_deployment_id_merge_into_a_copy() {
    let request = object(vec![(
        "metadata",
        guarded_metadata("dep-1", json!(["pii"])),
    )]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!(["block"])),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&guarded_metadata("dep-1", json!(["pii", "block"])))
    );
    assert_eq!(
        request.get("metadata"),
        Some(&guarded_metadata("dep-1", json!(["pii"])))
    );
}

#[test]
fn model_guardrails_wrap_a_bare_string_from_the_deployment() {
    let request = object(vec![("metadata", model_info(json!("dep-1")))]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!("block")),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&guarded_metadata("dep-1", json!(["block"])))
    );
}

#[test]
fn model_guardrails_wrap_a_bare_string_from_the_alias_group() {
    let request = object(vec![("model", json!("alias"))]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!("pii"), json!(["block"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![("guardrails", json!(["pii", "block"]))]))
    );
}

#[test]
fn model_guardrails_without_a_deployment_setting_leave_the_data() {
    let request = object(vec![("metadata", model_info(json!("dep-1")))]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::from([("dep-1".to_owned(), Deployment { guardrails: None })]),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_with_an_empty_list_still_normalize_the_metadata() {
    let request = object(vec![("metadata", guarded_metadata("dep-1", json!("pii")))]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!([])),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&guarded_metadata("dep-1", json!(["pii"])))
    );
}

#[test]
fn model_guardrails_with_an_unknown_id_leave_the_data() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", model_info(json!("missing"))),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_treat_a_non_string_id_as_an_unknown_deployment() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", model_info(json!(7_i64))),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_ignore_the_id_when_client_model_info_is_untrusted() {
    let request = object(vec![
        ("model", json!("alias")),
        ("metadata", model_info(json!("dep-1"))),
    ]);
    let lookup = FakeDeployments {
        by_id: deployments_by_id("dep-1", json!(["block"])),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), false, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&guarded_metadata("dep-1", json!(["pii"])))
    );
}

#[test]
fn model_guardrails_union_the_alias_group_for_the_team() {
    let request = object(vec![
        ("model", json!("ignored")),
        (
            "metadata",
            json_object(vec![("user_api_key_team_id", json!("team-a"))]),
        ),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name(
            "alias",
            Some("team-a"),
            vec![json!(["pii"]), json!(["block", "pii"])],
        ),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, Some("alias")).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![
            ("user_api_key_team_id", json!("team-a")),
            ("guardrails", json!(["pii", "block"])),
        ]))
    );
}

#[test]
fn model_guardrails_read_the_team_from_litellm_metadata() {
    let request = object(vec![
        ("model", json!("alias")),
        (
            "litellm_metadata",
            json_object(vec![("user_api_key_team_id", json!("team-b"))]),
        ),
    ]);
    let distinct = deployments_by_name("alias", Some("team-b"), vec![json!(["pii"])]);
    let other = deployments_by_name("alias", None, vec![json!(["block"])]);
    let by_name = distinct.into_iter().chain(other).collect();
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name,
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![("guardrails", json!(["pii"]))]))
    );
}

#[test]
fn model_guardrails_treat_a_non_string_team_id_as_unset() {
    let request = object(vec![
        ("model", json!("alias")),
        (
            "metadata",
            json_object(vec![("user_api_key_team_id", json!(7_i64))]),
        ),
    ]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("alias", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![
            ("user_api_key_team_id", json!(7_i64)),
            ("guardrails", json!(["pii"])),
        ]))
    );
}

#[rstest]
#[case::missing_model(object(vec![]))]
#[case::non_string_model(object(vec![("model", json!(7_i64))]))]
#[case::empty_model(object(vec![("model", json!(""))]))]
fn model_guardrails_without_an_alias_leave_the_data(#[case] request: JsonMap) {
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: deployments_by_name("", None, vec![json!(["pii"])]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[test]
fn model_guardrails_leave_the_data_when_the_alias_group_carries_none() {
    let request = object(vec![("model", json!("alias"))]);
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: BTreeMap::from([(
            ("alias".to_owned(), None),
            vec![Deployment { guardrails: None }],
        )]),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let merged = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap();
    assert_eq!(merged, request);
}

#[rstest]
#[case::metadata(object(vec![("metadata", json!([1_i64, 2_i64]))]), "metadata")]
#[case::litellm_metadata(object(vec![("litellm_metadata", json!("text"))]), "litellm_metadata")]
#[case::model_info(
    object(vec![(
        "metadata",
        json_object(vec![("model_info", json!(5_i64))]),
    )]),
    "metadata.model_info"
)]
fn model_guardrails_name_the_misplaced_object(#[case] request: JsonMap, #[case] field: &str) {
    let lookup = FakeDeployments {
        by_id: BTreeMap::new(),
        by_name: BTreeMap::new(),
    };
    let finder: &dyn DeploymentGuardrails = &lookup;
    let error = merge_model_level_guardrails(&request, Some(finder), true, None).unwrap_err();
    assert_eq!(error.to_string(), format!("'{field}' must be an object"));
}

#[test]
fn guardrail_merge_creates_metadata_and_unions_names() {
    let request = object(vec![
        ("model", json!("gpt-4o")),
        ("metadata", json_object(vec![("team", json!("acme"))])),
    ]);
    let merged = merge_guardrails_with_existing(&request, &json!(["pii", "block"])).unwrap();
    assert_eq!(
        merged,
        object(vec![
            ("model", json!("gpt-4o")),
            (
                "metadata",
                json_object(vec![
                    ("team", json!("acme")),
                    ("guardrails", json!(["pii", "block"])),
                ]),
            ),
        ])
    );
}

#[test]
fn guardrail_merge_wraps_scalars_on_both_sides() {
    let request = object(vec![(
        "metadata",
        json_object(vec![("guardrails", json!("pii"))]),
    )]);
    let merged = merge_guardrails_with_existing(&request, &json!("block")).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![("guardrails", json!(["pii", "block"]))]))
    );
}

#[test]
fn guardrail_merge_deduplicates_names() {
    let request = object(vec![(
        "metadata",
        json_object(vec![
            ("guardrails", json!(["pii", "block"])),
            ("team", json!("acme")),
        ]),
    )]);
    let merged =
        merge_guardrails_with_existing(&request, &json!(["block", "pii", "moderation"])).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![
            ("guardrails", json!(["pii", "block", "moderation"])),
            ("team", json!("acme")),
        ]))
    );
}

#[test]
fn guardrail_merge_keeps_a_nested_entry_once() {
    let nested = json_object(vec![("name", json!("pii"))]);
    let request = object(vec![(
        "metadata",
        json_object(vec![("guardrails", JsonValue::Array(vec![nested.clone()]))]),
    )]);
    let extra = JsonValue::Array(vec![nested, json!("block")]);
    let merged = merge_guardrails_with_existing(&request, &extra).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![(
            "guardrails",
            JsonValue::Array(vec![
                json_object(vec![("name", json!("pii"))]),
                json!("block"),
            ]),
        )]))
    );
}

#[test]
fn guardrail_merge_treats_a_null_model_level_list_as_empty() {
    let request = object(vec![]);
    let merged = merge_guardrails_with_existing(&request, &JsonValue::Null).unwrap();
    assert_eq!(
        merged,
        object(vec![(
            "metadata",
            json_object(vec![("guardrails", json!([]))]),
        )])
    );
}

#[test]
fn guardrail_merge_treats_a_null_metadata_as_absent() {
    let request = object(vec![("metadata", JsonValue::Null)]);
    let merged = merge_guardrails_with_existing(&request, &json!(["pii"])).unwrap();
    assert_eq!(
        merged,
        object(vec![(
            "metadata",
            json_object(vec![("guardrails", json!(["pii"]))]),
        )])
    );
}

#[test]
fn guardrail_merge_never_changes_the_given_request() {
    let request = object(vec![(
        "metadata",
        json_object(vec![("guardrails", json!(["pii"]))]),
    )]);
    let merged = merge_guardrails_with_existing(&request, &json!(["block"])).unwrap();
    assert_eq!(
        merged.get("metadata"),
        Some(&json_object(vec![("guardrails", json!(["pii", "block"]))]))
    );
    assert_eq!(
        request.get("metadata"),
        Some(&json_object(vec![("guardrails", json!(["pii"]))]))
    );
}

#[test]
fn guardrail_merge_rejects_a_non_object_metadata() {
    let request = object(vec![("metadata", json!(7_i64))]);
    let error = merge_guardrails_with_existing(&request, &json!(["pii"])).unwrap_err();
    assert_eq!(error.to_string(), "'metadata' must be an object");
}
