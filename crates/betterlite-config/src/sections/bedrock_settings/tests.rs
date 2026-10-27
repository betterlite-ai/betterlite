#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[rstest::rstest]
#[case::loads_aws_access_key_id("aws_access_key_id")]
#[case::loads_aws_secret_access_key("aws_secret_access_key")]
#[case::loads_aws_session_token("aws_session_token")]
#[case::loads_aws_region_name("aws_region_name")]
#[case::loads_aws_session_name("aws_session_name")]
#[case::loads_aws_profile_name("aws_profile_name")]
#[case::loads_aws_role_name("aws_role_name")]
#[case::loads_aws_web_identity_token("aws_web_identity_token")]
#[case::loads_aws_sts_endpoint("aws_sts_endpoint")]
#[case::loads_aws_external_id("aws_external_id")]
#[case::loads_aws_role_arn("aws_role_arn")]
#[case::loads_aws_web_identity_token_file("aws_web_identity_token_file")]
#[case::loads_aws_bearer_token_bedrock("aws_bearer_token_bedrock")]
#[case::loads_aws_session_tags("aws_session_tags")]
#[case::rejects_unrelated_key("unrelated")]
fn bedrock_settings_config_cases(#[case] field: &str) {
    if field == "unrelated" {
        let error = ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load("litellm_settings:\n  unrelated: true\n")
        .unwrap_err();
        assert!(error.to_string().contains("unrelated"));
        return;
    }
    let source = format!("litellm_settings:\n  {field}: configured\n");
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap();
    if field == "aws_session_tags" {
        assert!(
            config
                .sections
                .bedrock_settings
                .litellm_settings
                .aws_session_tags
                .is_some()
        );
    } else {
        assert!(
            config
                .sections
                .bedrock_settings
                .litellm_settings
                .deployable(field)
        );
    }
}
