use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::model_reload::dedupe::deduplicate_router_models;

use super::support::{deployment, deployment_without_id};

#[test]
fn duplicate_name_retained() {
    let models = [
        deployment("shared", "first", "openai/gpt-4o", true),
        deployment("shared", "second", "openai/gpt-4o", true),
        deployment("shared", "first", "openai/gpt-4o", true),
        deployment_without_id("shared"),
        deployment("other", "third", "openai/gpt-4o", true),
    ];
    let unique = deduplicate_router_models(&models);
    let names: Vec<&str> = unique
        .iter()
        .map(|model| model.model_name.as_str())
        .collect();
    let kept: Vec<Option<&str>> = unique.iter().map(DeploymentSpec::id).collect();
    assert_eq!(names, vec!["shared", "shared", "other"]);
    assert_eq!(kept, vec![Some("first"), Some("second"), Some("third")]);
}
