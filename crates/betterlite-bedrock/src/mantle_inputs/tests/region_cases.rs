use crate::mantle_inputs::error::MantleInputError;
use crate::mantle_inputs::region::{resolve_region, split_region_prefix};

use super::support::globals;

pub(super) fn mantle_region_prefix() {
    assert_eq!(
        split_region_prefix("us-east-1/anthropic.claude"),
        (Some("us-east-1"), "anthropic.claude")
    );
    assert_eq!(
        split_region_prefix("us-east-1/eu-west-1/x"),
        (Some("us-east-1"), "eu-west-1/x")
    );
    assert_eq!(split_region_prefix("us-east-1/"), (Some("us-east-1"), ""));
    assert_eq!(
        split_region_prefix("us-gov-west-1/x"),
        (Some("us-gov-west-1"), "x")
    );
    assert_eq!(split_region_prefix("sa-east-1/x"), (Some("sa-east-1"), "x"));
    assert_eq!(
        split_region_prefix("ap-northeast-3/x"),
        (Some("ap-northeast-3"), "x")
    );
    assert_eq!(split_region_prefix("eu-west-2"), (None, "eu-west-2"));
    assert_eq!(split_region_prefix("us-east/x"), (None, "us-east/x"));
    assert_eq!(split_region_prefix("US-EAST-1/x"), (None, "US-EAST-1/x"));
    assert_eq!(
        split_region_prefix("unknown/region"),
        (None, "unknown/region")
    );
    // the mantle and cross-region spellings are not stripped here
    assert_eq!(
        split_region_prefix("bedrock_mantle/us-east-1/x"),
        (None, "bedrock_mantle/us-east-1/x")
    );
    assert_eq!(
        split_region_prefix("global.anthropic.claude"),
        (None, "global.anthropic.claude")
    );
    assert_eq!(split_region_prefix("/x"), (None, "/x"));
    assert_eq!(split_region_prefix(""), (None, ""));
}

fn resolved(
    aws_region_name: Option<&str>,
    api_base: Option<&str>,
    environment: &[(&str, &str)],
) -> String {
    resolve_region(aws_region_name, api_base, &globals(environment)).unwrap()
}

fn rejected(
    aws_region_name: Option<&str>,
    api_base: Option<&str>,
    environment: &[(&str, &str)],
) -> bool {
    matches!(
        resolve_region(aws_region_name, api_base, &globals(environment)),
        Err(MantleInputError::Region(_))
    )
}

pub(super) fn mantle_region_resolution() {
    explicit_region_cases();
    host_cases();
    environment_cases();
}

fn explicit_region_cases() {
    assert_eq!(resolved(None, None, &[]), "us-east-1");
    assert_eq!(resolved(Some("eu-central-1"), None, &[]), "eu-central-1");
    assert!(rejected(Some("US-EAST-1"), None, &[]));
    assert!(rejected(Some("eu_west_1"), None, &[]));
    assert!(rejected(Some("us east 1"), None, &[]));
    // LiteLLM's `\A[a-z0-9-]+\Z` accepts a bare hyphen, and the resolver returns it unchanged
    assert_eq!(resolved(Some("-"), None, &[]), "-");
    // an empty explicit region is ignored rather than rejected
    assert_eq!(
        resolved(
            Some(""),
            Some("https://bedrock-mantle.eu-west-2.api.aws"),
            &[]
        ),
        "eu-west-2"
    );
    assert_eq!(
        resolved(
            Some("ca-central-1"),
            Some("https://bedrock-mantle.eu-west-2.api.aws"),
            &[]
        ),
        "ca-central-1"
    );
    assert!(rejected(
        Some("BAD_REGION"),
        Some("https://bedrock-mantle.eu-west-2.api.aws"),
        &[]
    ));
}

fn host_cases() {
    let host = "https://bedrock-mantle.eu-west-2.api.aws";
    assert_eq!(resolved(None, Some(host), &[]), "eu-west-2");
    assert_eq!(
        resolved(None, Some(format!("{host}/v1").as_str()), &[]),
        "eu-west-2"
    );
    assert_eq!(
        resolved(None, Some(format!("{host}/").as_str()), &[]),
        "eu-west-2"
    );
    assert_eq!(
        resolved(None, Some(format!("{host}//").as_str()), &[]),
        "eu-west-2"
    );
    assert_eq!(
        resolved(None, Some("http://bedrock-mantle.eu-west-2.api.aws"), &[]),
        "eu-west-2"
    );
    assert_eq!(
        resolved(None, Some("https://BEDROCK-MANTLE.US-EAST-1.API.AWS"), &[]),
        "US-EAST-1"
    );
    // the captured host label is returned unchanged, without region validation
    assert_eq!(
        resolved(None, Some("https://bedrock-mantle.US_East.api.aws"), &[]),
        "US_East"
    );
    assert_eq!(
        resolved(
            None,
            Some("https://bedrock-mantle.eu-west-2.api.aws\n"),
            &[]
        ),
        "eu-west-2"
    );
    for_unmatched_hosts();
}

fn for_unmatched_hosts() {
    assert_eq!(
        resolved(
            None,
            Some("https://bedrock-mantle.eu-west-2.api.aws:443"),
            &[]
        ),
        "us-east-1"
    );
    assert_eq!(
        resolved(
            None,
            Some("https://bedrock-mantle.eu-west-2.api.aws?a=1"),
            &[]
        ),
        "us-east-1"
    );
    assert_eq!(
        resolved(
            None,
            Some("https://x.bedrock-mantle.eu-west-2.api.aws"),
            &[]
        ),
        "us-east-1"
    );
    assert_eq!(
        resolved(None, Some("ftp://bedrock-mantle.eu-west-2.api.aws"), &[]),
        "us-east-1"
    );
}

fn environment_cases() {
    environment_base_cases();
    region_fallback_cases();
}

fn environment_base_cases() {
    let environment = [(
        "BEDROCK_MANTLE_API_BASE",
        "https://bedrock-mantle.sa-east-1.api.aws",
    )];
    assert_eq!(resolved(None, None, &environment), "sa-east-1");
    assert_eq!(resolved(None, Some(""), &environment), "sa-east-1");
    // a nonempty explicit base replaces the environment base before host matching
    assert_eq!(
        resolved(None, Some("https://elsewhere"), &environment),
        "us-east-1"
    );
}

fn region_fallback_cases() {
    assert_eq!(
        resolved(
            None,
            None,
            &[
                ("BEDROCK_MANTLE_REGION", "ap-south-1"),
                ("AWS_REGION_NAME", "eu-west-1"),
                ("AWS_REGION", "sa-east-1"),
            ]
        ),
        "ap-south-1"
    );
    empty_mantle_region_cases();
}

fn empty_mantle_region_cases() {
    assert_eq!(
        resolved(
            None,
            None,
            &[
                ("BEDROCK_MANTLE_REGION", ""),
                ("AWS_REGION_NAME", "eu-west-1"),
                ("AWS_REGION", "sa-east-1"),
            ]
        ),
        "eu-west-1"
    );
    assert_eq!(
        resolved(
            None,
            None,
            &[
                ("BEDROCK_MANTLE_REGION", ""),
                ("AWS_REGION_NAME", ""),
                ("AWS_REGION", "sa-east-1"),
            ]
        ),
        "sa-east-1"
    );
    assert_eq!(
        resolved(
            None,
            None,
            &[
                ("BEDROCK_MANTLE_REGION", ""),
                ("AWS_REGION_NAME", ""),
                ("AWS_REGION", ""),
            ]
        ),
        "us-east-1"
    );
}
