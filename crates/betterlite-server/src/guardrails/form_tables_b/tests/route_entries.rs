use serde_json::json;

use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::core_tables::core_tables;
use crate::guardrails::provider_forms::project::project_model;

const MAX_DEPTH: usize = 100;

#[test]
fn tool_permission_matches_the_fixed_route_entry() {
    let (_, core) = core_tables()
        .into_iter()
        .find(|(key, _)| *key == "tool_permission")
        .expect("the core tables hold the fixed tool_permission entry");
    let (_, registry) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "tool_permission")
        .expect("the registry tables hold the tool_permission entry");
    let mut core_fields =
        project_model(&core, MAX_DEPTH).expect("the core entry nests below the limit");
    core_fields.insert(
        "ui_friendly_name".to_owned(),
        core.ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    let mut registry_fields =
        project_model(&registry, MAX_DEPTH).expect("the registry entry nests below the limit");
    registry_fields.insert(
        "ui_friendly_name".to_owned(),
        registry
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(core_fields), json!(registry_fields));
}
