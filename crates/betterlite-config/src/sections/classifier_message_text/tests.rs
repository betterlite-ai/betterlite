#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::classifier_message_text::{MarkerPair, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.classifier_message_text)
        .map_err(|error| error.to_string())
}

fn markers(fields: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n        reminder_markers:\n{fields}"
    )
}

#[test]
fn a_deployment_without_the_key_keeps_its_index() {
    let source =
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![None],
        }
    );
}

#[test]
fn an_explicit_null_takes_the_declared_default() {
    let source = markers("          null\n");
    assert_eq!(
        settings(&source, &[]).unwrap(),
        Settings {
            deployments: vec![None],
        }
    );
}

#[test]
fn markers_are_trimmed_and_lowered() {
    let source = markers(
        "          - open: \" <System-Reminder> \"\n            close: \" </SYSTEM-REMINDER> \"\n",
    );
    assert_eq!(
        settings(&source, &[]).unwrap(),
        Settings {
            deployments: vec![Some(vec![MarkerPair {
                open: "<system-reminder>".to_owned(),
                close: "</system-reminder>".to_owned(),
            }])],
        }
    );
}

#[test]
fn an_environment_reference_resolves_before_normalizing() {
    let source = markers("          - open: os.environ/OPEN\n            close: \"</close>\"\n");
    let resolved = settings(&source, &[("OPEN", " <Open> ")]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Some(vec![MarkerPair {
            open: "<open>".to_owned(),
            close: "</close>".to_owned(),
        }])]
    );
}

#[test]
fn an_empty_marker_list_is_rejected() {
    let source = markers("          []\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("reminder_markers"), "{error}");
}

#[test]
fn a_blank_marker_is_rejected() {
    let source = markers("          - open: \"   \"\n            close: \"</close>\"\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("reminder_markers.0"), "{error}");
}

#[test]
fn a_blank_close_is_rejected() {
    let source = markers("          - open: \"<open>\"\n            close: \"   \"\n");
    assert!(settings(&source, &[]).is_err());
}

#[test]
fn equal_markers_are_rejected_after_normalizing() {
    let source = markers("          - open: \"<same>\"\n            close: \" <SAME> \"\n");
    assert!(settings(&source, &[]).is_err());
}

#[test]
fn pair_siblings_do_not_become_unsupported_keys() {
    let source = markers(
        "          - open: \"<open>\"\n            close: \"</close>\"\n            label: probe\n",
    );
    assert!(settings(&source, &[]).is_ok());
}
