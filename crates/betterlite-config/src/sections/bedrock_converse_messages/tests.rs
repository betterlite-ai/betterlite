#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<Settings, ConfigError> {
    load_with(source, &[])
}

fn load_with(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.bedrock_converse_messages)
}

fn deployment(fields: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, {fields}}}\n"
    )
}

fn global(fields: &str) -> String {
    format!("litellm_settings: {{{fields}}}\n")
}

#[derive(Clone, Copy, Debug)]
struct Flags {
    deployment: bool,
    global: bool,
}

fn both(flags: Flags) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, bedrock_neutralize_orphaned_tool_blocks: {deployment}}}\nlitellm_settings: {{bedrock_neutralize_orphaned_tool_blocks: {global}}}\n",
        deployment = flags.deployment,
        global = flags.global
    )
}

fn assert_flag(flags: Flags) {
    let default = load("").unwrap();
    assert!(
        default.flags.litellm_settings.is_none(),
        "the setting defaults to unset"
    );
    assert!(default.flags.deployments.is_empty());
    assert!(default.resolve(0), "the setting defaults to true");
    assert_each_source_alone(flags);
    assert_deployment_precedence(flags);
}

fn assert_each_source_alone(flags: Flags) {
    let deployment_value = flags.deployment;
    let only_deployment = load(&deployment(&format!(
        "bedrock_neutralize_orphaned_tool_blocks: {deployment_value}"
    )))
    .unwrap();
    assert_eq!(only_deployment.flags.deployments, [Some(deployment_value)]);
    assert!(only_deployment.flags.litellm_settings.is_none());
    assert_eq!(only_deployment.resolve(0), deployment_value);
    assert!(only_deployment.resolve(1));

    let global_value = flags.global;
    let only_global = load(&global(&format!(
        "bedrock_neutralize_orphaned_tool_blocks: {global_value}"
    )))
    .unwrap();
    assert!(only_global.flags.deployments.is_empty());
    assert_eq!(only_global.flags.litellm_settings, Some(global_value));
    assert_eq!(only_global.resolve(0), global_value);
}

fn assert_deployment_precedence(flags: Flags) {
    let configured = load(&both(flags)).unwrap();
    assert_eq!(configured.flags.litellm_settings, Some(flags.global));
    assert_eq!(
        configured.resolve(0),
        flags.deployment,
        "the deployment value wins"
    );
    assert_eq!(
        configured.resolve(1),
        flags.global,
        "the global value applies without a deployment"
    );
}

fn environment_references() {
    environment_values();
    environment_errors();
}

fn environment_values() {
    let read = |value: &str| {
        load_with(
            &deployment("bedrock_neutralize_orphaned_tool_blocks: os.environ/NEUTRALIZE_ORPHANS"),
            &[("NEUTRALIZE_ORPHANS", value)],
        )
    };
    assert_eq!(
        read("false").unwrap().flags.deployments,
        [Some(false)],
        "an os.environ reference reads into the field"
    );
    assert_eq!(read(" Yes ").unwrap().flags.deployments, [Some(true)]);
    assert_eq!(
        read("").unwrap().flags.deployments,
        [None],
        "an empty environment value counts as unset"
    );
    let empty_global = load_with(
        &global("bedrock_neutralize_orphaned_tool_blocks: os.environ/NEUTRALIZE_ORPHANS"),
        &[("NEUTRALIZE_ORPHANS", "")],
    )
    .unwrap();
    assert!(empty_global.flags.litellm_settings.is_none());
    assert!(empty_global.resolve(0));
}

fn environment_errors() {
    let missing = load(&deployment(
        "bedrock_neutralize_orphaned_tool_blocks: os.environ/ABSENT_NEUTRALIZE_ORPHANS",
    ))
    .unwrap_err();
    assert!(
        matches!(
            &missing,
            ConfigError::MissingEnvironment { variable, path }
                if variable == "ABSENT_NEUTRALIZE_ORPHANS"
                    && path == "model_list.0.litellm_params.bedrock_neutralize_orphaned_tool_blocks"
        ),
        "{missing}"
    );
    let invalid = load_with(
        &deployment("bedrock_neutralize_orphaned_tool_blocks: os.environ/NEUTRALIZE_ORPHANS"),
        &[("NEUTRALIZE_ORPHANS", "maybe")],
    )
    .unwrap_err();
    assert!(
        matches!(&invalid, ConfigError::Environment { .. }),
        "{invalid}"
    );
}

fn rejects_unrelated_keys() {
    let error = load(&deployment("bedrock_neutralize_unrelated: true")).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.bedrock_neutralize_unrelated"]),
        "{error}"
    );
    let global_error = load("litellm_settings: {bedrock_neutralize_orphans: true}\n").unwrap_err();
    assert!(
        matches!(&global_error, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.bedrock_neutralize_orphans"]),
        "{global_error}"
    );
}

#[derive(Clone, Copy, Debug)]
enum Field {
    Neutralize,
    RejectsUnrelatedKey,
}

#[rstest]
#[case::loads_bedrock_neutralize_orphaned_tool_blocks(Field::Neutralize)]
#[case::rejects_unrelated_key(Field::RejectsUnrelatedKey)]
fn bedrock_converse_messages_config_cases(#[case] field: Field) {
    match field {
        Field::Neutralize => {
            assert_flag(Flags {
                deployment: false,
                global: true,
            });
            assert_flag(Flags {
                deployment: true,
                global: false,
            });
            environment_references();
        }
        Field::RejectsUnrelatedKey => rejects_unrelated_keys(),
    }
}

#[test]
fn the_flags_stay_behind_one_pointer() {
    assert_eq!(size_of::<Settings>(), size_of::<usize>());
}
