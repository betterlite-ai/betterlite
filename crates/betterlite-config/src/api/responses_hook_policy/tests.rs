#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;

use crate::agentic_loop_ceiling::AgenticLoopCeilingError;
use crate::loader::{ConfigError, ConfigLoader};

const ACCEPTED: &str = "\
model_list:
- model_name: set
  litellm_params:
    model: openai/gpt-4o
    max_agentic_loops: 5
- model_name: unset
  litellm_params:
    model: openai/gpt-4o
- model_name: nullish
  litellm_params:
    model: openai/gpt-4o
    max_agentic_loops: null
- model_name: float
  litellm_params:
    model: openai/gpt-4o
    max_agentic_loops: 5.0
- model_name: padded
  litellm_params:
    model: openai/gpt-4o
    max_agentic_loops: ' 5 '
- model_name: signed
  litellm_params:
    model: openai/gpt-4o
    max_agentic_loops: '+5'
- model_name: underscored
  litellm_params:
    model: openai/gpt-4o
    max_agentic_loops: '1_000'
";

const FIELD_PATH: &str = "model_list.0.litellm_params.max_agentic_loops";
const FIELD: &str = "litellm_params.max_agentic_loops";

fn deployment(ceiling: &str) -> String {
    format!(
        "model_list:\n- model_name: only\n  litellm_params:\n    model: openai/gpt-4o\n    max_agentic_loops: {ceiling}\n"
    )
}

fn load(source: &str, lookup: BTreeMap<String, String>) -> Result<Vec<u32>, ConfigError> {
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.responses_hook_policy.deployments)
}

#[test]
fn loads_agentic_loop_ceiling() {
    assert_eq!(
        load(ACCEPTED, BTreeMap::new()).unwrap(),
        [5, 3, 3, 5, 5, 5, 1000]
    );
    invalid_values();
    environment_references();
    unknown_sibling_key();
}

fn invalid_values() {
    let field = FIELD.to_owned();
    let cases = [
        (
            "true",
            AgenticLoopCeilingError::NotWholeNumber {
                field: field.clone(),
            },
        ),
        (
            "0",
            AgenticLoopCeilingError::BelowMinimum {
                field: field.clone(),
                value: 0,
            },
        ),
        (
            "-1",
            AgenticLoopCeilingError::BelowMinimum {
                field: field.clone(),
                value: -1,
            },
        ),
        (
            "1.5",
            AgenticLoopCeilingError::NotWholeNumber {
                field: field.clone(),
            },
        ),
        ("three", AgenticLoopCeilingError::NotWholeNumber { field }),
    ];
    for (ceiling, expected) in cases {
        let error = load(&deployment(ceiling), BTreeMap::new()).unwrap_err();
        assert_eq!(error.category(), "invalid_agentic_loop_ceiling");
        let ConfigError::AgenticLoopCeiling { path, source } = &error else {
            panic!("expected an agentic loop ceiling error for {ceiling}, got {error:?}");
        };
        assert_eq!(path, FIELD_PATH);
        assert_eq!(source, &expected);
        assert_eq!(
            error.source().unwrap().to_string(),
            expected.to_string(),
            "the validator error stays the source for {ceiling}"
        );
    }
    let zero = load(&deployment("0"), BTreeMap::new()).unwrap_err();
    assert_eq!(
        zero.to_string(),
        "invalid agentic loop ceiling at model_list.0.litellm_params.max_agentic_loops: litellm_params.max_agentic_loops must be at least 1"
    );
}

fn environment_references() {
    let reference = deployment("os.environ/MAX_LOOPS");
    let lookup = BTreeMap::from([("MAX_LOOPS".to_owned(), "7".to_owned())]);
    assert_eq!(load(&reference, lookup).unwrap(), [7]);
    let error = load(&reference, BTreeMap::new()).unwrap_err();
    let ConfigError::MissingEnvironment { variable, path } = &error else {
        panic!("expected a missing environment variable, got {error:?}");
    };
    assert_eq!(variable.as_str(), "MAX_LOOPS");
    assert_eq!(path.as_str(), FIELD_PATH);
    let words = BTreeMap::from([("MAX_LOOPS".to_owned(), "zero".to_owned())]);
    let invalid = load(&reference, words).unwrap_err();
    assert!(matches!(
        invalid,
        ConfigError::AgenticLoopCeiling {
            source: AgenticLoopCeilingError::NotWholeNumber { .. },
            ..
        }
    ));
}

fn unknown_sibling_key() {
    let typo = deployment("5").replace("max_agentic_loops", "max_agentic_loop");
    let error = load(&typo, BTreeMap::new()).unwrap_err();
    let ConfigError::Unsupported { paths } = &error else {
        panic!("expected an unsupported key, got {error:?}");
    };
    assert_eq!(
        paths,
        &["model_list.0.litellm_params.max_agentic_loop".to_owned()]
    );
}
