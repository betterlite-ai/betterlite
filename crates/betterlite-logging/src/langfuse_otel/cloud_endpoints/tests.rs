#![cfg(test)]

use rstest::rstest;

use super::{LANGFUSE_CLOUD_EU_ENDPOINT, LANGFUSE_CLOUD_US_ENDPOINT};

#[rstest]
#[case::us_cloud_default(
    LANGFUSE_CLOUD_US_ENDPOINT,
    "https://us.cloud.langfuse.com/api/public/otel"
)]
#[case::eu_cloud(
    LANGFUSE_CLOUD_EU_ENDPOINT,
    "https://cloud.langfuse.com/api/public/otel"
)]
fn cloud_endpoints_are_the_documented_otlp_urls(#[case] endpoint: &str, #[case] expected: &str) {
    assert_eq!(endpoint, expected);
}
