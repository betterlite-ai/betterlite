#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(
    params: &str,
    settings: &str,
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let source = format!(
        "model_list:\n- model_name: agent\n  litellm_params: {{model: bedrock/agent/AGENT/ALIAS{params}}}\nlitellm_settings: {{{settings}}}\n"
    );
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(&source)?;
    Ok(config.sections.bedrock_services_invoke_agent)
}

fn session_id(settings: &Settings) -> Option<String> {
    settings.session_id.clone()
}

#[rstest]
#[case::loads_sessionid(check_session_id)]
fn invoke_agent_config_cases(#[case] check: fn()) {
    check();
}

const NO_ENVIRONMENT: &[(&str, &str)] = &[];
const ENVIRONMENT: &[(&str, &str)] = &[("INVOKE_AGENT_SESSION_ID", "from-environment")];

fn check_session_id() {
    for (params, settings, environment, expected) in [
        ("", "", NO_ENVIRONMENT, None),
        (
            "",
            "sessionID: global-value",
            NO_ENVIRONMENT,
            Some("global-value"),
        ),
        (
            ", sessionID: deployment-value",
            "",
            NO_ENVIRONMENT,
            Some("deployment-value"),
        ),
        (
            ", sessionID: deployment-value",
            "sessionID: global-value",
            NO_ENVIRONMENT,
            Some("deployment-value"),
        ),
        ("", "sessionID: \"\"", NO_ENVIRONMENT, Some("")),
        (
            "",
            "sessionID: os.environ/INVOKE_AGENT_SESSION_ID",
            ENVIRONMENT,
            Some("from-environment"),
        ),
    ] {
        assert_eq!(
            session_id(&load(params, settings, environment).unwrap()),
            expected.map(str::to_owned),
            "{settings}"
        );
    }
    check_session_id_rejects_unknown_keys();
}

fn check_session_id_rejects_unknown_keys() {
    let error = load(", sessionID_unrelated: 1", "", &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == [String::from("model_list.0.litellm_params.sessionID_unrelated")]),
        "{error}"
    );
}

#[test]
fn second_deployment_wins_over_the_first() {
    let source = "model_list:\n- model_name: a\n  litellm_params: {model: bedrock/agent/a/b, sessionID: first}\n- model_name: b\n  litellm_params: {model: bedrock/agent/a/b, sessionID: second}\n";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config.sections.bedrock_services_invoke_agent.session_id,
        Some("second".to_owned())
    );
}
