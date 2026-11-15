use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::project::project_model;

use super::expected::{expected, projected};

const MAX_DEPTH: usize = 100;

#[test]
fn agent_365_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "agent_365")
        .expect("agent_365 has a table");
    assert_eq!(model.ui_friendly_name, Some("Microsoft Agent 365"));
    let fields = project_model(&model, MAX_DEPTH).expect("agent_365 nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/agent_365.json"))
    );
}

#[test]
fn aim_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "aim")
        .expect("aim has a table");
    assert_eq!(model.ui_friendly_name, Some("AIM Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("aim nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/aim.json"))
    );
}

#[test]
fn akto_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "akto")
        .expect("akto has a table");
    assert_eq!(model.ui_friendly_name, Some("Akto"));
    let fields = project_model(&model, MAX_DEPTH).expect("akto nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/akto.json"))
    );
}

#[test]
fn cato_networks_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "cato_networks")
        .expect("cato_networks has a table");
    assert_eq!(model.ui_friendly_name, Some("Cato Networks Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("cato_networks nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/cato_networks.json"))
    );
}

#[test]
fn cisco_ai_defense_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "cisco_ai_defense")
        .expect("cisco_ai_defense has a table");
    assert_eq!(model.ui_friendly_name, Some("Cisco AI Defense"));
    let fields = project_model(&model, MAX_DEPTH).expect("cisco_ai_defense nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/cisco_ai_defense.json"))
    );
}

#[test]
fn ibm_guardrails_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "ibm_guardrails")
        .expect("ibm_guardrails has a table");
    assert_eq!(model.ui_friendly_name, Some("IBM Guardrails Detector"));
    let fields = project_model(&model, MAX_DEPTH).expect("ibm_guardrails nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/ibm_guardrails.json"))
    );
}

#[test]
fn model_armor_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "model_armor")
        .expect("model_armor has a table");
    assert_eq!(model.ui_friendly_name, Some("Google Cloud Model Armor"));
    let fields = project_model(&model, MAX_DEPTH).expect("model_armor nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/model_armor.json"))
    );
}

#[test]
fn pangea_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "pangea")
        .expect("pangea has a table");
    assert_eq!(model.ui_friendly_name, Some("Pangea Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("pangea nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/pangea.json"))
    );
}

#[test]
fn singulr_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "singulr")
        .expect("singulr has a table");
    assert_eq!(model.ui_friendly_name, Some("Singulr"));
    let fields = project_model(&model, MAX_DEPTH).expect("singulr nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/singulr.json"))
    );
}
