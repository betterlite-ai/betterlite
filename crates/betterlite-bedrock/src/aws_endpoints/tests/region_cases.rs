use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_endpoints::region::{
    self, LlmRegion, non_llm_region_name, region_from_model_arn, validate_region,
};

const ANTHROPIC_ARN: &str = "arn:aws:bedrock:us-east-1:123456789012:model/anthropic.claude";

fn input<'text>(
    explicit: Option<&'text str>,
    model: Option<&'text str>,
    model_id: Option<&'text str>,
) -> LlmRegion<'text> {
    LlmRegion {
        explicit,
        model,
        model_id,
        environment_region_name: None,
        environment_region: None,
        session_region: None,
    }
}

fn assert_region(model: &str, expected: &str) {
    assert_eq!(
        region_from_model_arn(Some(model)),
        Some(expected.to_owned()),
        "{model}"
    );
}

fn assert_no_region(model: Option<&str>) {
    assert_eq!(region_from_model_arn(model), None, "{model:?}");
}

pub(super) fn model_arn_region() {
    assert_region(ANTHROPIC_ARN, "us-east-1");
    assert_region("arn:aws-cn:bedrock:cn-north-1:1:model/x", "cn-north-1");
    assert_region(
        "arn:aws-us-gov:bedrock:us-gov-west-1:1:model/x",
        "us-gov-west-1",
    );
    assert_region("arn:aws:bedrock:us-east-1", "us-east-1");
    assert_region("xarn:aws:bedrock:eu-west-1:1:y", "eu-west-1");
    assert_region("arn:aws:bedrock:us-east-1x:1:y", "us-east-1x");
    assert_no_region(Some("arn:aws:bedrock:INVALID-Region:1:x"));
    assert_no_region(Some("arn:aws:bedrock::1:x"));
    assert_no_region(Some("arn:aws:bedrock:eu_west_1:1:x"));
    assert_no_region(Some("arn:aws:s3:us-east-1:1:model/x"));
    assert_no_region(Some("model/anthropic.claude"));
    assert_no_region(None);
}

pub(super) fn explicit_region_precedence() {
    let mut configured = input(
        Some("eu-central-1"),
        Some(ANTHROPIC_ARN),
        Some("other-model"),
    );
    configured.environment_region_name = Some("ap-south-1");
    configured.environment_region = Some("ap-northeast-1");
    configured.session_region = Some("sa-east-1");
    assert_eq!(region::llm_region_name(configured).unwrap(), "eu-central-1");
    assert_eq!(
        region::llm_region_name(input(Some("us-west-2"), None, None)).unwrap(),
        "us-west-2"
    );
    assert_eq!(
        region::llm_region_name(input(Some("cn-north-1"), Some("x"), Some("y"))).unwrap(),
        "cn-north-1"
    );
}

fn config_session_region(environment: &[(&str, &str)]) -> Option<String> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("{}")
        .unwrap()
        .sections
        .bedrock_aws_endpoints
        .session_region
}

pub(super) fn environment_profile_fallback() {
    let profile = config_session_region(&[("AWS_DEFAULT_REGION", "ap-south-1")]);
    let mut configured = input(None, None, None);
    configured.session_region = profile.as_deref();
    assert_eq!(region::llm_region_name(configured).unwrap(), "ap-south-1");

    let missing = config_session_region(&[]);
    let mut absent = input(None, None, None);
    absent.session_region = missing.as_deref();
    assert_eq!(region::llm_region_name(absent).unwrap(), "us-west-2");

    let empty = config_session_region(&[("AWS_DEFAULT_REGION", "")]);
    let mut empty_session = input(None, None, None);
    empty_session.session_region = empty.as_deref();
    assert_eq!(region::llm_region_name(empty_session).unwrap(), "us-west-2");

    let mut name_beats_session = input(None, None, None);
    name_beats_session.environment_region_name = Some("eu-west-1");
    name_beats_session.session_region = profile.as_deref();
    assert_eq!(
        region::llm_region_name(name_beats_session).unwrap(),
        "eu-west-1"
    );
}

pub(super) fn non_llm_region_precedence() {
    assert_eq!(
        non_llm_region_name(
            Some("us-east-1"),
            Some("ap-south-1"),
            Some("ap-northeast-1")
        )
        .unwrap(),
        "us-east-1"
    );
    assert_eq!(
        non_llm_region_name(None, Some("ap-south-1"), Some("ap-northeast-1")).unwrap(),
        "ap-northeast-1"
    );
    assert_eq!(
        non_llm_region_name(None, Some("ap-south-1"), None).unwrap(),
        "ap-south-1"
    );
    assert_eq!(non_llm_region_name(None, None, None).unwrap(), "us-west-2");
    assert!(matches!(
        non_llm_region_name(None, Some(""), None),
        Err(AwsEndpointError::Region { .. })
    ));
    assert!(matches!(
        non_llm_region_name(None, Some("ap-south-1"), Some("")),
        Err(AwsEndpointError::Region { .. })
    ));
    assert!(matches!(
        non_llm_region_name(Some(""), None, None),
        Err(AwsEndpointError::Region { .. })
    ));
}

pub(super) fn invalid_region() {
    for valid in ["us-east-1", "us-gov-west-1", "cn-north-1", "-", "123"] {
        assert!(validate_region(Some(valid)).is_ok(), "{valid}");
    }
    validate_region(None).unwrap();
    for invalid in ["", "US-EAST-1", "us_east_1", "us east 1", "us-east_1"] {
        assert!(
            matches!(
                validate_region(Some(invalid)),
                Err(AwsEndpointError::Region { .. })
            ),
            "{invalid}"
        );
    }
}

pub(super) fn llm_region_precedence() {
    environment_precedence();
    model_id_override();
    session_and_explicit_region();
}

fn model_id_override() {
    let mut arn_override = input(
        None,
        Some(ANTHROPIC_ARN),
        Some("arn:aws:bedrock:eu-west-1:1:model/x"),
    );
    arn_override.environment_region_name = Some("ap-south-1");
    assert_eq!(region::llm_region_name(arn_override).unwrap(), "eu-west-1");
}

fn environment_precedence() {
    let mut invalid_override = input(None, Some(ANTHROPIC_ARN), Some("model-1"));
    invalid_override.environment_region_name = Some("ap-south-1");
    assert_eq!(
        region::llm_region_name(invalid_override).unwrap(),
        "ap-south-1"
    );

    let mut arn_beats_environment = input(None, Some(ANTHROPIC_ARN), None);
    arn_beats_environment.environment_region_name = Some("ap-south-1");
    arn_beats_environment.environment_region = Some("ap-northeast-1");
    assert_eq!(
        region::llm_region_name(arn_beats_environment).unwrap(),
        "us-east-1"
    );

    let mut name_first = input(None, None, None);
    name_first.environment_region_name = Some("ap-south-1");
    name_first.environment_region = Some("ap-northeast-1");
    assert_eq!(region::llm_region_name(name_first).unwrap(), "ap-south-1");

    let mut region_second = input(None, None, None);
    region_second.environment_region = Some("ap-northeast-1");
    assert_eq!(
        region::llm_region_name(region_second).unwrap(),
        "ap-northeast-1"
    );

    let mut empty_name = input(None, None, None);
    empty_name.environment_region_name = Some("");
    assert!(matches!(
        region::llm_region_name(empty_name),
        Err(AwsEndpointError::Region { .. })
    ));

    let mut empty_region = input(None, None, None);
    empty_region.environment_region = Some("");
    assert!(matches!(
        region::llm_region_name(empty_region),
        Err(AwsEndpointError::Region { .. })
    ));
}

fn session_and_explicit_region() {
    let mut session = input(None, None, None);
    session.session_region = Some("us-west-1");
    assert_eq!(region::llm_region_name(session).unwrap(), "us-west-1");

    let mut empty_session = input(None, None, None);
    empty_session.session_region = Some("");
    assert_eq!(region::llm_region_name(empty_session).unwrap(), "us-west-2");

    let mut explicit_wins = input(Some("us-east-1"), None, None);
    explicit_wins.environment_region_name = Some("");
    assert_eq!(region::llm_region_name(explicit_wins).unwrap(), "us-east-1");

    assert!(matches!(
        region::llm_region_name(input(Some(""), None, None)),
        Err(AwsEndpointError::Region { .. })
    ));
}
