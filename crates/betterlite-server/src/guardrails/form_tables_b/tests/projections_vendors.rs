use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::project::project_model;

use super::expected::{expected, projected};

const MAX_DEPTH: usize = 100;

#[test]
fn aporia_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "aporia")
        .expect("aporia has a table");
    assert_eq!(model.ui_friendly_name, Some("Aporia AI"));
    let fields = project_model(&model, MAX_DEPTH).expect("aporia nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/aporia.json"))
    );
}

#[test]
fn compresr_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "compresr")
        .expect("compresr has a table");
    assert_eq!(
        model.ui_friendly_name,
        Some("Compresr (context compression)")
    );
    let fields = project_model(&model, MAX_DEPTH).expect("compresr nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/compresr.json"))
    );
}

#[test]
fn deepkeep_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "deepkeep")
        .expect("deepkeep has a table");
    assert_eq!(model.ui_friendly_name, Some("DeepKeep AI Firewall"));
    let fields = project_model(&model, MAX_DEPTH).expect("deepkeep nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/deepkeep.json"))
    );
}

#[test]
fn enkryptai_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "enkryptai")
        .expect("enkryptai has a table");
    assert_eq!(model.ui_friendly_name, Some("EnkryptAI"));
    let fields = project_model(&model, MAX_DEPTH).expect("enkryptai nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/enkryptai.json"))
    );
}

#[test]
fn lasso_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "lasso")
        .expect("lasso has a table");
    assert_eq!(model.ui_friendly_name, Some("Lasso Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("lasso nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/lasso.json"))
    );
}

#[test]
fn onyx_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "onyx")
        .expect("onyx has a table");
    assert_eq!(model.ui_friendly_name, Some("Onyx Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("onyx nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/onyx.json"))
    );
}

#[test]
fn ovalix_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "ovalix")
        .expect("ovalix has a table");
    assert_eq!(model.ui_friendly_name, Some("Ovalix Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("ovalix nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/ovalix.json"))
    );
}

#[test]
fn pillar_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "pillar")
        .expect("pillar has a table");
    assert_eq!(model.ui_friendly_name, Some("Pillar Guardrail"));
    let fields = project_model(&model, MAX_DEPTH).expect("pillar nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/pillar.json"))
    );
}

#[test]
fn prompt_security_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "prompt_security")
        .expect("prompt_security has a table");
    assert_eq!(model.ui_friendly_name, Some("Prompt Security"));
    let fields = project_model(&model, MAX_DEPTH).expect("prompt_security nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/prompt_security.json"))
    );
}

#[test]
fn repelloai_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "repelloai")
        .expect("repelloai has a table");
    assert_eq!(model.ui_friendly_name, Some("RepelloAI Argus"));
    let fields = project_model(&model, MAX_DEPTH).expect("repelloai nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/repelloai.json"))
    );
}

#[test]
fn straiker_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "straiker")
        .expect("straiker has a table");
    assert_eq!(model.ui_friendly_name, Some("Straiker"));
    let fields = project_model(&model, MAX_DEPTH).expect("straiker nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/straiker.json"))
    );
}

#[test]
fn vigil_guard_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "vigil_guard")
        .expect("vigil_guard has a table");
    assert_eq!(model.ui_friendly_name, Some("Vigil Guard"));
    let fields = project_model(&model, MAX_DEPTH).expect("vigil_guard nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/vigil_guard.json"))
    );
}
