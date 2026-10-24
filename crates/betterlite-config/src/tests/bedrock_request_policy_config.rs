use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn fields(source: &str) -> Result<Vec<String>, ConfigError> {
    let mut environment = BTreeMap::new();
    environment.insert("FIELDS".to_owned(), "user_api_key_hash".to_owned());
    let config = ConfigLoader {
        lookup: environment,
    }
    .load(source)?;
    Ok(config.litellm_settings.bedrock_metadata_fields)
}

#[rstest]
#[case::loads_bedrock_request_metadata_fields(loads_bedrock_request_metadata_fields)]
#[case::rejects_unrelated_key(rejects_unrelated_key)]
fn bedrock_request_policy_config_cases(#[case] check: fn()) {
    check();
}

fn loads_bedrock_request_metadata_fields() {
    let configured = "litellm_settings:\n  bedrock_request_metadata_fields: [user_api_key_hash, 1, true, ~, '', os.environ/FIELDS, user_api_key_hash]\n";
    let expected = [
        "user_api_key_hash",
        "1",
        "True",
        "None",
        "",
        "os.environ/FIELDS",
        "user_api_key_hash",
    ];
    assert_eq!(fields(configured).unwrap(), expected);
    assert_eq!(fields("").unwrap(), Vec::<String>::new());
    let null = "litellm_settings:\n  bedrock_request_metadata_fields: ~\n";
    assert_eq!(fields(null).unwrap(), Vec::<String>::new());
    let scalar = "litellm_settings:\n  bedrock_request_metadata_fields: os.environ/FIELDS\n";
    assert!(matches!(fields(scalar), Err(ConfigError::Yaml { .. })));
    let float = "litellm_settings:\n  bedrock_request_metadata_fields: [1.5]\n";
    assert!(matches!(fields(float), Err(ConfigError::Yaml { .. })));
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  bedrock_request_metadata_fields: [user_api_key_hash]\n  bedrock_request_metadata_field: [x]\n";
    let paths = vec!["litellm_settings.bedrock_request_metadata_field".to_owned()];
    assert!(
        matches!(fields(source), Err(ConfigError::Unsupported { paths: found }) if found == paths)
    );
}
