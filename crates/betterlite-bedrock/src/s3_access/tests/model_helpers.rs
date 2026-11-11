use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use rstest::rstest;

use crate::s3_access::S3AccessError;
use crate::s3_access::models::{bedrock_model_id, invoke_route_model, strip_llm_routing_prefix};
use crate::s3_access::uri::parse_s3_uri;

fn context<'context>(
    catalog: &'context ModelCatalog,
    names: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: names,
        custom_providers: names,
        environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

pub(super) fn strip_with_empty_context(model: &str) -> String {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    strip_llm_routing_prefix(model, &context(&catalog, &names, &environment))
}

pub(super) fn invoke_with_empty_context(model: &str) -> String {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    invoke_route_model(model, &context(&catalog, &names, &environment))
}

#[rstest]
#[case::one_exact_prefix("bedrock/anthropic.claude-3-sonnet", "anthropic.claude-3-sonnet")]
#[case::empty_model_after_the_prefix("bedrock/", "")]
#[case::only_the_exact_prefix("bedrock", "bedrock")]
#[case::wrong_prefix("bedrockx/model", "bedrockx/model")]
#[case::second_prefix_is_kept("bedrock/bedrock/model", "bedrock/model")]
#[case::route_prefix_is_kept("invoke/foo", "invoke/foo")]
#[case::empty("", "")]
fn bedrock_model_id_cases(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(bedrock_model_id(model), expected);
}

#[rstest]
#[case::explicit_provider("bedrock/anthropic.claude-3-sonnet", "anthropic.claude-3-sonnet")]
#[case::openai_provider("openai/gpt-4o", "gpt-4o")]
#[case::route_prefix_resolves_to_nothing("invoke/foo", "invoke/foo")]
#[case::unresolvable_name_keeps_the_model("amazon.titan-text-express", "amazon.titan-text-express")]
#[case::empty("", "")]
fn strip_llm_routing_prefix_cases(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(strip_with_empty_context(model), expected);
}

#[rstest]
#[case::provider_prefix("bedrock/foo", "invoke/foo")]
#[case::existing_invoke_prefix("invoke/foo", "invoke/foo")]
#[case::bare_model("foo", "invoke/foo")]
#[case::provider_then_invoke("bedrock/invoke/foo", "invoke/foo")]
#[case::empty("", "invoke/")]
fn invoke_route_model_cases(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(invoke_with_empty_context(model), expected);
}

#[rstest]
#[case::bucket_and_key("s3://bucket/key/path", Ok(("bucket", "key/path")))]
#[case::trailing_slash("s3://bucket/", Ok(("bucket", "")))]
#[case::empty_bucket("s3:///key", Ok(("", "key")))]
#[case::empty_key("s3://bucket", Err(()))]
#[case::wrong_scheme("ftp://bucket/key", Err(()))]
#[case::uppercase_scheme("S3://bucket/key", Err(()))]
#[case::percent_encoded("s3%3A%2F%2Fbucket%2Fkey", Err(()))]
#[case::traversal_is_accepted_here("s3://bucket/a/../b", Ok(("bucket", "a/../b")))]
#[case::empty("", Err(()))]
fn parse_s3_uri_cases(#[case] uri: &str, #[case] expected: Result<(&str, &str), ()>) {
    let wanted = expected.map(|(bucket, key)| (bucket.to_owned(), key.to_owned()));
    assert_eq!(parse_s3_uri(uri).map_err(|_error| ()), wanted);
}

#[test]
fn the_invalid_uri_error_names_the_input() {
    let error = parse_s3_uri("ftp://bucket/key").expect_err("the scheme is wrong");
    assert_eq!(error.to_string(), "Invalid S3 URI format: ftp://bucket/key");
    assert!(matches!(error, S3AccessError::InvalidS3Uri { .. }));
}
