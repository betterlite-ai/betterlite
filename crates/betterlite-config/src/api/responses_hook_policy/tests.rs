#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, parse_view};
use crate::sections::Section as _;

use super::Raw;

const ACCEPTED: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: 5
- litellm_params: {}
- litellm_params:
    max_agentic_loops: null
- litellm_params:
    max_agentic_loops: 5.0
- litellm_params:
    max_agentic_loops: ' 5 '
- litellm_params:
    max_agentic_loops: '+5'
- litellm_params:
    max_agentic_loops: '1_000'
";

const ENVIRONMENT: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: os.environ/MAX_LOOPS
";

const BOOLEAN: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: true
";

const ZERO: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: 0
";

const NEGATIVE: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: -1
";

const FRACTION: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: 1.5
";

const TEXT: &str = "\
model_list:
- litellm_params:
    max_agentic_loops: 'three'
";

const PATH: &str = "model_list.0.litellm_params.max_agentic_loops";

fn load(source: &str, lookup: BTreeMap<String, String>) -> Result<Vec<u32>, ConfigError> {
    let (raw, ignored) = parse_view::<Raw>(source)?;
    assert!(
        ignored.is_empty(),
        "the section view claims every key it reads: {ignored:?}"
    );
    let settings = raw.convert(&lookup)?;
    Ok(settings.deployments)
}

#[test]
fn loads_agentic_loop_ceiling() {
    assert_eq!(
        load(ACCEPTED, BTreeMap::new()).unwrap(),
        [5, 3, 3, 5, 5, 5, 1000]
    );
    invalid_values();
    environment_references();
}

fn invalid_values() {
    for source in [BOOLEAN, ZERO, NEGATIVE, FRACTION, TEXT] {
        let error = load(source, BTreeMap::new()).unwrap_err();
        let path = if let ConfigError::InvalidScalar { path, .. } = &error {
            path.clone()
        } else {
            panic!("expected an invalid scalar for {source:?}, got {error:?}");
        };
        assert_eq!(path, PATH);
    }
}

fn environment_references() {
    let lookup = BTreeMap::from([("MAX_LOOPS".to_owned(), "7".to_owned())]);
    assert_eq!(load(ENVIRONMENT, lookup).unwrap(), [7]);
    let error = load(ENVIRONMENT, BTreeMap::new()).unwrap_err();
    if let ConfigError::MissingEnvironment { variable, path } = &error {
        assert_eq!(variable.as_str(), "MAX_LOOPS");
        assert_eq!(path.as_str(), PATH);
    } else {
        panic!("expected a missing environment variable, got {error:?}");
    }
}
