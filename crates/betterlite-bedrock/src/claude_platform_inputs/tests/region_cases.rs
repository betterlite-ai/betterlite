use super::support;
use crate::claude_platform_inputs::endpoint::{UrlInputs, complete_url, resolve_region};
use crate::claude_platform_inputs::error::PlatformInputError;
use crate::model_routes::{ParamMap, ParamValue};

fn region(optional: &ParamMap, environment: &[(&str, &str)]) -> Result<String, PlatformInputError> {
    resolve_region(optional, &support::aws_endpoints(environment))
}

fn url(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    optional: &ParamMap,
    environment: &[(&str, &str)],
) -> Result<String, PlatformInputError> {
    let settings = support::settings(environment);
    complete_url(
        &UrlInputs {
            api_base,
            litellm_api_base,
            optional_params: optional,
        },
        &settings.environment,
        &support::aws_endpoints(environment),
    )
}

fn empty_params() -> ParamMap {
    support::params(&[])
}

pub(super) fn explicit_region_required() {
    missing_region_is_authentication_error();
    environment_regions_apply();
    region_precedence_decides();
    empty_and_invalid_regions_fail_validation();
    scalars_encode_before_validation();
}

fn missing_region_is_authentication_error() {
    let error = region(&empty_params(), &[]).unwrap_err();
    assert!(
        matches!(error, PlatformInputError::MissingRegion),
        "{error}"
    );
    assert_eq!(error.status_code(), Some(401));
    assert!(
        error
            .to_string()
            .contains("Missing AWS region for Claude Platform on AWS")
    );
}

fn environment_regions_apply() {
    let explicit = support::params(&[("aws_region_name", support::text("eu-west-1"))]);
    assert_eq!(region(&explicit, &[]).unwrap(), "eu-west-1");
    assert_eq!(
        region(&empty_params(), &[("AWS_REGION_NAME", "us-east-2")]).unwrap(),
        "us-east-2"
    );
    assert_eq!(
        region(&empty_params(), &[("AWS_REGION", "ap-south-1")]).unwrap(),
        "ap-south-1"
    );
    assert_eq!(
        region(&empty_params(), &[("AWS_DEFAULT_REGION", "sa-east-1")]).unwrap(),
        "sa-east-1"
    );
    assert_eq!(
        region(
            &support::params(&[("aws_region_name", support::text(""))]),
            &[("AWS_REGION_NAME", "us-east-2")]
        )
        .unwrap(),
        "us-east-2"
    );
}

fn region_precedence_decides() {
    assert_eq!(
        region(
            &empty_params(),
            &[
                ("AWS_REGION_NAME", "us-east-2"),
                ("AWS_REGION", "ap-south-1"),
                ("AWS_DEFAULT_REGION", "sa-east-1"),
            ]
        )
        .unwrap(),
        "us-east-2"
    );
    assert_eq!(
        region(
            &empty_params(),
            &[
                ("AWS_REGION", "ap-south-1"),
                ("AWS_DEFAULT_REGION", "sa-east-1"),
            ]
        )
        .unwrap(),
        "ap-south-1"
    );
}

fn empty_and_invalid_regions_fail_validation() {
    let null_region = support::params(&[("aws_region_name", ParamValue::Null)]);
    let error = region(&null_region, &[]).unwrap_err();
    assert!(
        matches!(error, PlatformInputError::MissingRegion),
        "{error}"
    );
    assert_eq!(
        error.to_string(),
        "Missing AWS region for Claude Platform on AWS. Pass `aws_region_name` or set a standard AWS region environment value."
    );
    let invalid = support::params(&[("aws_region_name", support::text("US-WEST-2"))]);
    let invalid_error = region(&invalid, &[]).unwrap_err();
    assert!(
        matches!(invalid_error, PlatformInputError::Region(_)),
        "{invalid_error}"
    );
    assert_eq!(invalid_error.status_code(), None);
    let empty_first_environment = region(&empty_params(), &[("AWS_REGION_NAME", "")]).unwrap_err();
    assert!(
        matches!(empty_first_environment, PlatformInputError::MissingRegion),
        "{empty_first_environment}"
    );
    let empty_final_environment = region(
        &empty_params(),
        &[("AWS_REGION", ""), ("AWS_DEFAULT_REGION", "")],
    )
    .unwrap_err();
    assert!(
        matches!(empty_final_environment, PlatformInputError::Region(_)),
        "{empty_final_environment}"
    );
    assert_eq!(empty_final_environment.status_code(), None);
}

fn scalars_encode_before_validation() {
    let number = support::params(&[("aws_region_name", support::integer(1))]);
    assert_eq!(region(&number, &[]).unwrap(), "1");
    let boolean = support::params(&[("aws_region_name", ParamValue::Bool(true))]);
    assert!(matches!(
        region(&boolean, &[]),
        Err(PlatformInputError::Region(_))
    ));
    let zero = support::params(&[("aws_region_name", support::integer(0))]);
    assert!(matches!(
        region(&zero, &[]),
        Err(PlatformInputError::MissingRegion)
    ));
}

pub(super) fn api_base_before_region() {
    explicit_bases_append_the_path();
    explicit_base_beats_global();
    environment_bases_apply();
    environment_base_url_wins();
    empty_base_needs_no_region();
    region_builds_the_default_base();
    supplied_base_skips_region_validation();
}

fn explicit_bases_append_the_path() {
    assert_eq!(
        url(Some("https://custom.example"), None, &empty_params(), &[]).unwrap(),
        "https://custom.example/v1/messages"
    );
    assert_eq!(
        url(Some("https://custom.example/"), None, &empty_params(), &[]).unwrap(),
        "https://custom.example/v1/messages"
    );
    assert_eq!(
        url(
            Some("https://custom.example///"),
            None,
            &empty_params(),
            &[]
        )
        .unwrap(),
        "https://custom.example/v1/messages"
    );
    assert_eq!(
        url(
            Some("https://custom.example/v1/messages"),
            None,
            &empty_params(),
            &[]
        )
        .unwrap(),
        "https://custom.example/v1/messages"
    );
    assert_eq!(
        url(
            Some("https://custom.example/v1/messages/"),
            None,
            &empty_params(),
            &[]
        )
        .unwrap(),
        "https://custom.example/v1/messages/v1/messages"
    );
}

fn explicit_base_beats_global() {
    assert_eq!(
        url(
            Some("https://explicit.example"),
            Some("https://global.example"),
            &empty_params(),
            &[]
        )
        .unwrap(),
        "https://explicit.example/v1/messages"
    );
    assert_eq!(
        url(None, Some("https://global.example"), &empty_params(), &[]).unwrap(),
        "https://global.example/v1/messages"
    );
    assert_eq!(
        url(
            Some(""),
            Some("https://global.example"),
            &empty_params(),
            &[("ANTHROPIC_AWS_BASE_URL", "https://base.example")]
        )
        .unwrap(),
        "https://global.example/v1/messages"
    );
}

fn environment_bases_apply() {
    assert_eq!(
        url(
            None,
            None,
            &empty_params(),
            &[("ANTHROPIC_AWS_BASE_URL", "https://base.example")]
        )
        .unwrap(),
        "https://base.example/v1/messages"
    );
    assert_eq!(
        url(
            None,
            None,
            &empty_params(),
            &[("ANTHROPIC_AWS_API_BASE", "https://api.example")]
        )
        .unwrap(),
        "https://api.example/v1/messages"
    );
}

fn environment_base_url_wins() {
    assert_eq!(
        url(
            None,
            None,
            &empty_params(),
            &[
                ("ANTHROPIC_AWS_BASE_URL", "https://base.example"),
                ("ANTHROPIC_AWS_API_BASE", "https://api.example"),
            ]
        )
        .unwrap(),
        "https://base.example/v1/messages"
    );
    assert_eq!(
        url(
            None,
            None,
            &empty_params(),
            &[
                ("ANTHROPIC_AWS_BASE_URL", ""),
                ("ANTHROPIC_AWS_API_BASE", "https://api.example"),
            ]
        )
        .unwrap(),
        "https://api.example/v1/messages"
    );
}

fn empty_base_needs_no_region() {
    assert_eq!(
        url(
            None,
            None,
            &empty_params(),
            &[("ANTHROPIC_AWS_API_BASE", "")]
        )
        .unwrap(),
        "/v1/messages"
    );
    let explicit_empty = url(Some(""), None, &empty_params(), &[]).unwrap_err();
    assert!(
        matches!(explicit_empty, PlatformInputError::MissingRegion),
        "{explicit_empty}"
    );
}

fn region_builds_the_default_base() {
    assert_eq!(
        url(
            None,
            None,
            &support::params(&[("aws_region_name", support::text("eu-west-1"))]),
            &[]
        )
        .unwrap(),
        "https://aws-external-anthropic.eu-west-1.api.aws/v1/messages"
    );
    assert_eq!(
        url(None, None, &empty_params(), &[("AWS_REGION", "us-east-2")]).unwrap(),
        "https://aws-external-anthropic.us-east-2.api.aws/v1/messages"
    );
    let missing = url(None, None, &empty_params(), &[]).unwrap_err();
    assert!(
        matches!(missing, PlatformInputError::MissingRegion),
        "{missing}"
    );
}

fn supplied_base_skips_region_validation() {
    assert_eq!(
        url(
            Some("https://custom.example"),
            None,
            &support::params(&[("aws_region_name", support::text("BAD_REGION"))]),
            &[]
        )
        .unwrap(),
        "https://custom.example/v1/messages"
    );
}
