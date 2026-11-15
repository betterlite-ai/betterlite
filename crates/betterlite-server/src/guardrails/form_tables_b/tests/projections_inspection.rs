use serde_json::json;

use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::project::project_model;

const MAX_DEPTH: usize = 100;

#[test]
fn agent_365_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "agent_365")
        .expect("agent_365 has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("agent_365 nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/agent_365.rs"));
}

#[test]
fn aim_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "aim")
        .expect("aim has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("aim nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/aim.rs"));
}

#[test]
fn akto_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "akto")
        .expect("akto has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("akto nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/akto.rs"));
}

#[test]
fn cato_networks_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "cato_networks")
        .expect("cato_networks has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("cato_networks nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/cato_networks.rs"));
}

#[test]
fn cisco_ai_defense_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "cisco_ai_defense")
        .expect("cisco_ai_defense has a table");
    let mut fields =
        project_model(&model, MAX_DEPTH).expect("cisco_ai_defense nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/cisco_ai_defense.rs"));
}

#[test]
fn ibm_guardrails_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "ibm_guardrails")
        .expect("ibm_guardrails has a table");
    let mut fields =
        project_model(&model, MAX_DEPTH).expect("ibm_guardrails nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/ibm_guardrails.rs"));
}

#[test]
fn model_armor_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "model_armor")
        .expect("model_armor has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("model_armor nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/model_armor.rs"));
}

#[test]
fn pangea_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "pangea")
        .expect("pangea has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("pangea nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/pangea.rs"));
}

#[test]
fn singulr_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "singulr")
        .expect("singulr has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("singulr nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/singulr.rs"));
}
