use betterlite_config::sections::claude_platform_inputs::PlatformInputs;
use indexmap::IndexMap;

use super::support;
use crate::claude_platform_inputs::error::PlatformInputError;
use crate::claude_platform_inputs::workspace::resolve_workspace_id;
use crate::model_routes::{ParamMap, ParamValue};

fn workspace(
    optional: &ParamMap,
    deployment: &PlatformInputs,
    global: &PlatformInputs,
    environment: &[(&str, &str)],
) -> Option<String> {
    let settings = support::settings(environment);
    resolve_workspace_id(optional, deployment, global, &settings.environment).unwrap()
}

pub(super) fn workspace_alias_precedence() {
    defaults_to_none();
    optional_beats_deployment();
    deployment_then_module_values();
    alias_order_decides();
    hyphen_deployment_and_global();
    underscore_chain_is_separate();
    falsy_values_continue_the_chain();
    environment_first_wins();
    environment_empty_values();
    scalars_render_like_str();
    null_is_absent();
    containers_are_rejected();
}

fn defaults_to_none() {
    let empty = support::empty_inputs();
    assert_eq!(workspace(&support::params(&[]), &empty, &empty, &[]), None);
}

fn optional_beats_deployment() {
    let empty = support::empty_inputs();
    let optional = support::params(&[("workspace_id", support::text("optional"))]);
    let mut deployment = support::empty_inputs();
    deployment.workspace_id = Some("deployment".to_owned());
    assert_eq!(
        workspace(&optional, &deployment, &empty, &[]),
        Some("optional".to_owned())
    );
    assert_eq!(
        workspace(&support::params(&[]), &deployment, &empty, &[]),
        Some("deployment".to_owned())
    );
}

fn deployment_then_module_values() {
    let empty = support::empty_inputs();
    let mut global = support::empty_inputs();
    global.workspace_id = Some("global".to_owned());
    assert_eq!(
        workspace(&support::params(&[]), &empty, &global, &[]),
        Some("global".to_owned())
    );
    let mut hidden = support::empty_inputs();
    hidden.workspace_id = Some(String::new());
    assert_eq!(
        workspace(&support::params(&[]), &hidden, &global, &[]),
        None
    );
}

fn alias_order_decides() {
    let empty = support::empty_inputs();
    let mut deployment = support::empty_inputs();
    deployment.workspace_id = Some("deployment".to_owned());
    let aws_optional = support::params(&[("aws_workspace_id", support::text("optional-aws"))]);
    assert_eq!(
        workspace(&aws_optional, &deployment, &empty, &[]),
        Some("deployment".to_owned())
    );
    assert_eq!(
        workspace(&aws_optional, &empty, &empty, &[]),
        Some("optional-aws".to_owned())
    );
    let mut aws_deployment = support::empty_inputs();
    aws_deployment.aws_workspace_id = Some("deployment-aws".to_owned());
    let hyphen_optional =
        support::params(&[("anthropic-workspace-id", support::text("optional-hyphen"))]);
    assert_eq!(
        workspace(&hyphen_optional, &aws_deployment, &empty, &[]),
        Some("deployment-aws".to_owned())
    );
}

fn hyphen_deployment_and_global() {
    let empty = support::empty_inputs();
    let mut hyphen_deployment = support::empty_inputs();
    hyphen_deployment.anthropic_hyphen_workspace_id = Some("deployment-hyphen".to_owned());
    assert_eq!(
        workspace(&support::params(&[]), &hyphen_deployment, &empty, &[]),
        Some("deployment-hyphen".to_owned())
    );
    let mut hyphen_global = support::empty_inputs();
    hyphen_global.anthropic_hyphen_workspace_id = Some("global-hyphen".to_owned());
    assert_eq!(
        workspace(&support::params(&[]), &empty, &hyphen_global, &[]),
        Some("global-hyphen".to_owned())
    );
    let mut empty_hyphen = support::empty_inputs();
    empty_hyphen.anthropic_hyphen_workspace_id = Some(String::new());
    assert_eq!(
        workspace(&support::params(&[]), &empty_hyphen, &hyphen_global, &[]),
        Some(String::new())
    );
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty_hyphen,
            &empty,
            &[("ANTHROPIC_AWS_WORKSPACE_ID", "env-workspace")]
        ),
        Some(String::new())
    );
}

fn underscore_chain_is_separate() {
    let empty = support::empty_inputs();
    let underscore = support::params(&[("anthropic_workspace_id", support::text("underscore"))]);
    assert_eq!(
        workspace(&underscore, &empty, &empty, &[]),
        Some("underscore".to_owned())
    );
    let both_chains = support::params(&[
        ("workspace_id", support::text("first")),
        ("anthropic_workspace_id", support::text("second")),
    ]);
    assert_eq!(
        workspace(&both_chains, &empty, &empty, &[]),
        Some("first".to_owned())
    );
    let mut underscore_deployment = support::empty_inputs();
    underscore_deployment.anthropic_workspace_id = Some("underscore-deployment".to_owned());
    assert_eq!(
        workspace(&support::params(&[]), &underscore_deployment, &empty, &[]),
        Some("underscore-deployment".to_owned())
    );
}

fn falsy_values_continue_the_chain() {
    let empty = support::empty_inputs();
    let empty_workspace = support::params(&[
        ("workspace_id", support::text("")),
        ("aws_workspace_id", support::text("aws")),
    ]);
    assert_eq!(
        workspace(&empty_workspace, &empty, &empty, &[]),
        Some("aws".to_owned())
    );
    let empty_hyphen_param = support::params(&[("anthropic-workspace-id", support::text(""))]);
    assert_eq!(
        workspace(
            &empty_hyphen_param,
            &empty,
            &empty,
            &[("ANTHROPIC_AWS_WORKSPACE_ID", "env-workspace")]
        ),
        Some("env-workspace".to_owned())
    );
    let empty_then_underscore = support::params(&[
        ("anthropic-workspace-id", support::text("")),
        ("anthropic_workspace_id", support::text("underscore")),
    ]);
    assert_eq!(
        workspace(&empty_then_underscore, &empty, &empty, &[]),
        Some("underscore".to_owned())
    );
    let zero = support::params(&[("anthropic-workspace-id", support::integer(0))]);
    assert_eq!(workspace(&zero, &empty, &empty, &[]), None);
    let false_flag = support::params(&[("anthropic-workspace-id", ParamValue::Bool(false))]);
    assert_eq!(workspace(&false_flag, &empty, &empty, &[]), None);
}

fn environment_first_wins() {
    let empty = support::empty_inputs();
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty,
            &empty,
            &[("ANTHROPIC_AWS_WORKSPACE_ID", "env-workspace")]
        ),
        Some("env-workspace".to_owned())
    );
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty,
            &empty,
            &[("ANTHROPIC_WORKSPACE_ID", "second-workspace")]
        ),
        Some("second-workspace".to_owned())
    );
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty,
            &empty,
            &[
                ("ANTHROPIC_AWS_WORKSPACE_ID", "first-workspace"),
                ("ANTHROPIC_WORKSPACE_ID", "second-workspace"),
            ]
        ),
        Some("first-workspace".to_owned())
    );
}

fn environment_empty_values() {
    let empty = support::empty_inputs();
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty,
            &empty,
            &[
                ("ANTHROPIC_AWS_WORKSPACE_ID", ""),
                ("ANTHROPIC_WORKSPACE_ID", "second-workspace"),
            ]
        ),
        Some("second-workspace".to_owned())
    );
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty,
            &empty,
            &[("ANTHROPIC_AWS_WORKSPACE_ID", "")]
        ),
        None
    );
    assert_eq!(
        workspace(
            &support::params(&[]),
            &empty,
            &empty,
            &[
                ("ANTHROPIC_AWS_WORKSPACE_ID", ""),
                ("ANTHROPIC_WORKSPACE_ID", ""),
            ]
        ),
        Some(String::new())
    );
}

fn scalars_render_like_str() {
    let empty = support::empty_inputs();
    let number = support::params(&[("anthropic-workspace-id", support::integer(5))]);
    assert_eq!(
        workspace(&number, &empty, &empty, &[]),
        Some("5".to_owned())
    );
    let flag = support::params(&[("workspace_id", ParamValue::Bool(true))]);
    assert_eq!(
        workspace(&flag, &empty, &empty, &[]),
        Some("True".to_owned())
    );
}

fn null_is_absent() {
    let empty = support::empty_inputs();
    let mut deployment = support::empty_inputs();
    deployment.workspace_id = Some("deployment".to_owned());
    let null_param = support::params(&[("workspace_id", ParamValue::Null)]);
    assert_eq!(
        workspace(&null_param, &deployment, &empty, &[]),
        Some("deployment".to_owned())
    );
}

fn containers_are_rejected() {
    let empty = support::empty_inputs();
    let list = support::params(&[(
        "anthropic-workspace-id",
        ParamValue::List(vec![support::text("a")]),
    )]);
    let mapping = support::params(&[(
        "anthropic-workspace-id",
        ParamValue::Mapping(IndexMap::from([("a".to_owned(), support::text("b"))])),
    )]);
    let settings = support::settings(&[]);
    let list_error =
        resolve_workspace_id(&list, &empty, &empty, &settings.environment).unwrap_err();
    assert!(
        matches!(list_error, PlatformInputError::ValueType { field } if field == "anthropic-workspace-id"),
        "{list_error}"
    );
    assert_eq!(list_error.status_code(), Some(400));
    let mapping_error =
        resolve_workspace_id(&mapping, &empty, &empty, &settings.environment).unwrap_err();
    assert!(
        matches!(mapping_error, PlatformInputError::ValueType { field } if field == "anthropic-workspace-id"),
        "{mapping_error}"
    );
    assert_eq!(mapping_error.status_code(), Some(400));
}
