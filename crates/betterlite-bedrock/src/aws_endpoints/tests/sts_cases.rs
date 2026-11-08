use betterlite_config::tls::Verification;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_endpoints::sts::{
    StsSettings, resolve_sts_region, sts_client_kwargs, sts_region_from_endpoint as parse_endpoint,
};

fn settings<'text>(
    endpoint: Option<&'text str>,
    aws_region: Option<&'text str>,
    aws_default_region: Option<&'text str>,
    configured_region: Option<&'text str>,
) -> StsSettings<'text> {
    StsSettings {
        endpoint,
        aws_region,
        aws_default_region,
        configured_region,
    }
}

fn assert_region(endpoint: Option<&str>, expected: Option<&str>) {
    assert_eq!(
        parse_endpoint(endpoint).unwrap().as_deref(),
        expected,
        "{endpoint:?}"
    );
}

pub(super) fn sts_region_from_endpoint() {
    endpoint_regions();
    endpoint_non_matches();
}

fn endpoint_regions() {
    let vpce = "https://vpce-abc123.sts.eu-west-1.vpce.amazonaws.com";
    let suffix = "https://sts.us-east-1.amazonaws.com.evil.example";
    let userinfo = "https://user:info@sts.eu-west-1.amazonaws.com/path?x=1";
    assert_region(
        Some("https://sts.us-east-1.amazonaws.com"),
        Some("us-east-1"),
    );
    assert_region(
        Some("https://sts-fips.us-east-1.amazonaws.com"),
        Some("us-east-1"),
    );
    assert_region(Some(vpce), Some("eu-west-1"));
    assert_region(
        Some("https://sts.cn-north-1.amazonaws.com.cn"),
        Some("cn-north-1"),
    );
    assert_region(
        Some("https://sts.us-gov-west-1.amazonaws.com"),
        Some("us-gov-west-1"),
    );
    assert_region(
        Some("https://STS.EU-WEST-1.AMAZONAWS.COM"),
        Some("eu-west-1"),
    );
    assert_region(Some(suffix), Some("us-east-1"));
    assert_region(Some(userinfo), Some("eu-west-1"));
}

fn endpoint_non_matches() {
    assert_region(Some("https://xsts.us-east-1.amazonaws.com"), None);
    assert_region(Some("https://example.com"), None);
    assert_region(Some("sts.eu-west-1.amazonaws.com"), None);
    assert_region(Some(""), None);
    assert_region(None, None);
    assert!(matches!(
        parse_endpoint(Some("https://[::1")),
        Err(AwsEndpointError::Endpoint { .. })
    ));
}

pub(super) fn sts_region_and_kwargs() {
    region_fallbacks();
    kwargs_shapes();
}

fn region_fallbacks() {
    let endpoint_region = settings(
        Some("https://sts.eu-west-1.amazonaws.com"),
        Some("us-east-1"),
        Some("us-west-2"),
        Some("ap-south-1"),
    );
    assert_eq!(
        resolve_sts_region(&endpoint_region).unwrap().as_deref(),
        Some("eu-west-1")
    );

    let region_env = settings(
        Some("https://example.com"),
        Some("us-east-1"),
        Some("us-west-2"),
        None,
    );
    assert_eq!(
        resolve_sts_region(&region_env).unwrap().as_deref(),
        Some("us-east-1")
    );

    let default_env = settings(None, Some(""), Some("us-west-2"), None);
    assert_eq!(
        resolve_sts_region(&default_env).unwrap().as_deref(),
        Some("us-west-2")
    );

    let configured = settings(None, Some(""), Some(""), Some("ap-south-1"));
    assert_eq!(
        resolve_sts_region(&configured).unwrap().as_deref(),
        Some("ap-south-1")
    );

    let empty_configured = settings(None, None, None, Some(""));
    assert_eq!(
        resolve_sts_region(&empty_configured).unwrap().as_deref(),
        Some("")
    );

    let nothing = settings(None, None, None, None);
    assert_eq!(resolve_sts_region(&nothing).unwrap(), None);
}

fn kwargs_shapes() {
    let endpoint_region = settings(
        Some("https://sts.eu-west-1.amazonaws.com"),
        Some("us-east-1"),
        Some("us-west-2"),
        Some("ap-south-1"),
    );
    let endpoint_kwargs = sts_client_kwargs(&endpoint_region, Verification::Enabled).unwrap();
    assert_eq!(endpoint_kwargs.verify, Verification::Enabled);
    assert_eq!(
        endpoint_kwargs.endpoint_url.as_deref(),
        Some("https://sts.eu-west-1.amazonaws.com")
    );
    assert_eq!(endpoint_kwargs.region_name.as_deref(), Some("eu-west-1"));

    let nothing = settings(None, None, None, None);
    let nothing_kwargs = sts_client_kwargs(&nothing, Verification::Disabled).unwrap();
    assert_eq!(nothing_kwargs.verify, Verification::Disabled);
    assert_eq!(nothing_kwargs.endpoint_url, None);
    assert_eq!(nothing_kwargs.region_name, None);

    let empty_endpoint = settings(Some(""), None, None, None);
    let empty_kwargs =
        sts_client_kwargs(&empty_endpoint, Verification::CaFile("ca.pem".to_owned())).unwrap();
    assert_eq!(
        empty_kwargs.verify,
        Verification::CaFile("ca.pem".to_owned())
    );
    assert_eq!(empty_kwargs.endpoint_url.as_deref(), Some(""));
    assert_eq!(empty_kwargs.region_name, None);
}
