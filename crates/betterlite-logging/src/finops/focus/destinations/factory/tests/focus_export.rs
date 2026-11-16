use crate::finops::focus::destinations::factory::DestinationProvider as _;
use crate::finops::focus::destinations::factory::destination::Destination;
use crate::finops::focus::destinations::factory::resolve::DestinationOverrides;

use super::support::{
    BUCKET, MAVVRIK_CONNECTION, MAVVRIK_ENDPOINT, MAVVRIK_KEY, PREFIX, VANTAGE_KEY, VANTAGE_TOKEN,
    destination_overrides, factory,
};

#[test]
fn dispatch_creates_an_s3_destination_for_s3() {
    let subject = factory(&[("FOCUS_S3_BUCKET_NAME", BUCKET)]);
    let destination = subject
        .create("s3", PREFIX, &DestinationOverrides::default())
        .expect("the s3 provider is supported");
    assert!(matches!(destination, Destination::S3(_)));
}

#[test]
fn dispatch_creates_a_vantage_destination_for_vantage() {
    let subject = factory(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
    ]);
    let destination = subject
        .create("vantage", PREFIX, &DestinationOverrides::default())
        .expect("the vantage provider is supported");
    assert!(matches!(destination, Destination::Vantage(_)));
}

#[test]
fn dispatch_creates_a_gcs_destination_for_gcs() {
    let subject = factory(&[("FOCUS_GCS_BUCKET_NAME", BUCKET)]);
    let destination = subject
        .create("gcs", PREFIX, &DestinationOverrides::default())
        .expect("the gcs provider is supported");
    assert!(matches!(destination, Destination::Gcs(_)));
}

#[test]
fn dispatch_creates_a_mavvrik_destination_for_mavvrik() {
    let subject = factory(&[
        ("MAVVRIK_API_KEY", MAVVRIK_KEY),
        ("MAVVRIK_API_ENDPOINT", MAVVRIK_ENDPOINT),
        ("MAVVRIK_CONNECTION_ID", MAVVRIK_CONNECTION),
    ]);
    let destination = subject
        .create("mavvrik", PREFIX, &DestinationOverrides::default())
        .expect("the mavvrik provider is supported");
    assert!(matches!(destination, Destination::Mavvrik(_)));
}

#[test]
fn dispatch_accepts_an_upper_case_provider() {
    let subject = factory(&[("FOCUS_S3_BUCKET_NAME", BUCKET)]);
    let destination = subject
        .create("S3", PREFIX, &DestinationOverrides::default())
        .expect("the provider name is matched case-insensitively");
    assert!(matches!(destination, Destination::S3(_)));
}

#[test]
fn dispatch_accepts_a_mixed_case_provider() {
    let subject = factory(&[
        ("VANTAGE_API_KEY", VANTAGE_KEY),
        ("VANTAGE_INTEGRATION_TOKEN", VANTAGE_TOKEN),
    ]);
    let destination = subject
        .create("VaNtAgE", PREFIX, &DestinationOverrides::default())
        .expect("the provider name is matched case-insensitively");
    assert!(matches!(destination, Destination::Vantage(_)));
}

#[test]
fn dispatch_reports_an_unsupported_provider() {
    let error = factory(&[])
        .create("bedrock", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the provider is not supported");
    assert_eq!(
        error.to_string(),
        "Provider 'bedrock' not supported for Focus export configuration"
    );
}

#[test]
fn dispatch_lowercases_the_unsupported_provider() {
    let error = factory(&[])
        .create("BEDROCK", PREFIX, &DestinationOverrides::default())
        .err()
        .expect("the provider is not supported");
    assert_eq!(
        error.to_string(),
        "Provider 'bedrock' not supported for Focus export configuration"
    );
}

#[test]
fn dispatch_uses_the_configuration_override() {
    let mut overrides = destination_overrides();
    overrides.bucket_name = Some(String::from("override-bucket"));
    overrides.region_name = Some(String::from("eu-west-1"));
    let subject = factory(&[
        ("FOCUS_S3_BUCKET_NAME", BUCKET),
        ("FOCUS_S3_REGION_NAME", "us-east-1"),
    ]);
    let destination = subject
        .create("s3", PREFIX, &overrides)
        .expect("the s3 provider is supported");
    let Destination::S3(s3_destination) = destination else {
        panic!("the factory did not build an S3 destination");
    };
    assert_eq!(s3_destination.bucket_name, "override-bucket");
    assert_eq!(
        s3_destination.client_options.region_name.as_deref(),
        Some("eu-west-1")
    );
}

#[test]
fn dispatch_passes_the_prefix_to_the_destination() {
    let subject = factory(&[("FOCUS_S3_BUCKET_NAME", BUCKET)]);
    let destination = subject
        .create("s3", PREFIX, &DestinationOverrides::default())
        .expect("the s3 provider is supported");
    let Destination::S3(s3_destination) = destination else {
        panic!("the factory did not build an S3 destination");
    };
    assert_eq!(s3_destination.prefix, PREFIX);
}
