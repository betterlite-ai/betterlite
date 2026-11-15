use serde_json::json;

use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::project::project_model;

const MAX_DEPTH: usize = 100;

#[test]
fn aporia_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "aporia")
        .expect("aporia has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("aporia nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/aporia.rs"));
}

#[test]
fn compresr_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "compresr")
        .expect("compresr has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("compresr nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/compresr.rs"));
}

#[test]
fn deepkeep_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "deepkeep")
        .expect("deepkeep has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("deepkeep nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/deepkeep.rs"));
}

#[test]
fn enkryptai_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "enkryptai")
        .expect("enkryptai has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("enkryptai nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/enkryptai.rs"));
}

#[test]
fn lasso_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "lasso")
        .expect("lasso has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("lasso nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/lasso.rs"));
}

#[test]
fn onyx_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "onyx")
        .expect("onyx has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("onyx nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/onyx.rs"));
}

#[test]
fn ovalix_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "ovalix")
        .expect("ovalix has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("ovalix nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/ovalix.rs"));
}

#[test]
fn pillar_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "pillar")
        .expect("pillar has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("pillar nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/pillar.rs"));
}

#[test]
fn prompt_security_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "prompt_security")
        .expect("prompt_security has a table");
    let mut fields =
        project_model(&model, MAX_DEPTH).expect("prompt_security nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/prompt_security.rs"));
}

#[test]
fn repelloai_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "repelloai")
        .expect("repelloai has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("repelloai nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/repelloai.rs"));
}

#[test]
fn straiker_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "straiker")
        .expect("straiker has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("straiker nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/straiker.rs"));
}

#[test]
fn vigil_guard_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "vigil_guard")
        .expect("vigil_guard has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("vigil_guard nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/vigil_guard.rs"));
}
