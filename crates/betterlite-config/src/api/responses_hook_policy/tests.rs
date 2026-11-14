#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

const DEPLOYMENT: &str =
    "model_list:\n- model_name: only\n  litellm_params:\n    model: openai/gpt-4o\n";
const FIELD_PATH: &str = "model_list.0.litellm_params.max_agentic_loops";

enum Expected {
    Ceilings(Vec<u32>),
    InvalidScalar { value: bool },
    MissingEnvironment,
}

fn load(source: &str, lookup: BTreeMap<String, String>) -> Result<Vec<u32>, ConfigError> {
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.responses_hook_policy.deployments)
}

#[rstest]
#[case::explicit_integer(Some("5"), BTreeMap::new(), Expected::Ceilings(vec![5]))]
#[case::explicit_float(Some("5.0"), BTreeMap::new(), Expected::Ceilings(vec![5]))]
#[case::padded_text(Some("' 5 '"), BTreeMap::new(), Expected::Ceilings(vec![5]))]
#[case::signed_text(Some("'+5'"), BTreeMap::new(), Expected::Ceilings(vec![5]))]
#[case::separated_text(Some("'1_000'"), BTreeMap::new(), Expected::Ceilings(vec![1000]))]
#[case::minimum(Some("1"), BTreeMap::new(), Expected::Ceilings(vec![1]))]
#[case::explicit_null(Some("null"), BTreeMap::new(), Expected::Ceilings(vec![3]))]
#[case::unset(None, BTreeMap::new(), Expected::Ceilings(vec![3]))]
#[case::environment(
    Some("os.environ/MAX_LOOPS"),
    BTreeMap::from([("MAX_LOOPS".to_owned(), "7".to_owned())]),
    Expected::Ceilings(vec![7])
)]
#[case::padded_environment(
    Some("os.environ/MAX_LOOPS"),
    BTreeMap::from([("MAX_LOOPS".to_owned(), " 7 ".to_owned())]),
    Expected::Ceilings(vec![7])
)]
#[case::separated_environment(
    Some("os.environ/MAX_LOOPS"),
    BTreeMap::from([("MAX_LOOPS".to_owned(), "1_000".to_owned())]),
    Expected::Ceilings(vec![1000])
)]
#[case::empty_environment(
    Some("os.environ/MAX_LOOPS"),
    BTreeMap::from([("MAX_LOOPS".to_owned(), "  ".to_owned())]),
    Expected::Ceilings(vec![3])
)]
#[case::missing_environment(
    Some("os.environ/MAX_LOOPS"),
    BTreeMap::new(),
    Expected::MissingEnvironment
)]
#[case::invalid_environment(
    Some("os.environ/MAX_LOOPS"),
    BTreeMap::from([("MAX_LOOPS".to_owned(), "zero".to_owned())]),
    Expected::InvalidScalar { value: false }
)]
#[case::boolean_true(Some("true"), BTreeMap::new(), Expected::InvalidScalar { value: true })]
#[case::boolean_false(Some("false"), BTreeMap::new(), Expected::InvalidScalar { value: false })]
#[case::zero(Some("0"), BTreeMap::new(), Expected::InvalidScalar { value: false })]
#[case::negative(Some("-1"), BTreeMap::new(), Expected::InvalidScalar { value: false })]
#[case::fraction(Some("1.5"), BTreeMap::new(), Expected::InvalidScalar { value: false })]
#[case::text(Some("three"), BTreeMap::new(), Expected::InvalidScalar { value: false })]
#[case::empty_text(Some("''"), BTreeMap::new(), Expected::InvalidScalar { value: false })]
fn loads_agentic_loop_ceiling(
    #[case] ceiling: Option<&str>,
    #[case] lookup: BTreeMap<String, String>,
    #[case] expected: Expected,
) {
    let source = match ceiling {
        None => DEPLOYMENT.to_owned(),
        Some(line) => format!("{DEPLOYMENT}    max_agentic_loops: {line}\n"),
    };
    let actual = load(&source, lookup);
    match expected {
        Expected::Ceilings(ceilings) => assert_eq!(actual.unwrap(), ceilings),
        Expected::InvalidScalar { value } => {
            let error = actual.unwrap_err();
            assert_eq!(error.category(), "invalid_scalar");
            let ConfigError::InvalidScalar {
                path,
                value: rejected,
            } = &error
            else {
                panic!("expected an invalid scalar, got {error:?}");
            };
            assert_eq!(path.as_str(), FIELD_PATH);
            assert_eq!(*rejected, value);
            assert_eq!(error.to_string(), format!("invalid scalar at {FIELD_PATH}"));
        }
        Expected::MissingEnvironment => {
            let error = actual.unwrap_err();
            let ConfigError::MissingEnvironment { variable, path } = &error else {
                panic!("expected a missing environment variable, got {error:?}");
            };
            assert_eq!(variable.as_str(), "MAX_LOOPS");
            assert_eq!(path.as_str(), FIELD_PATH);
        }
    }
}

#[test]
fn every_deployment_keeps_its_own_ceiling() {
    let source = "\
model_list:
- model_name: five
  litellm_params:
    max_agentic_loops: 5
- model_name: unset
  litellm_params: {}
";
    assert_eq!(load(source, BTreeMap::new()).unwrap(), [5, 3]);
}

#[test]
fn an_unknown_sibling_key_stays_unsupported() {
    let source = format!("{DEPLOYMENT}    max_agentic_loop: 5\n");
    let error = load(&source, BTreeMap::new()).unwrap_err();
    let ConfigError::Unsupported { paths } = &error else {
        panic!("expected an unsupported key, got {error:?}");
    };
    assert_eq!(
        paths,
        &["model_list.0.litellm_params.max_agentic_loop".to_owned()]
    );
}
