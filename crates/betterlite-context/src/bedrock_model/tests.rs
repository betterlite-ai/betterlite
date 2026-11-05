#![cfg(test)]

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use rstest::rstest;

use super::model_info::{LOCAL_MODEL_COST_MAP, LazyBinding, cached_catalog};
use super::regions::{AmazonBedrockGlobalConfig, CROSS_REGION_PREFIXES};
use super::runtime_arn::encode_runtime_modelid_arn;
use super::{
    extract_model_from_arn, get_bedrock_base_model, split_bedrock_region_path,
    strip_bedrock_routing_prefix, strip_throughput_suffix,
};

fn region_groups_case() {
    let regions = AmazonBedrockGlobalConfig.get_all_regions();
    assert_eq!(
        regions,
        vec![
            "us-east-1",
            "us-east-2",
            "us-west-1",
            "us-west-2",
            "us-gov-east-1",
            "us-gov-west-1",
            "eu-west-1",
            "eu-west-2",
            "eu-west-3",
            "eu-central-1",
            "eu-central-2",
            "eu-south-1",
            "eu-south-2",
            "eu-north-1",
            "ap-northeast-1",
            "ap-northeast-2",
            "ap-northeast-3",
            "ap-south-1",
            "ap-south-2",
            "ap-southeast-1",
            "ap-southeast-2",
            "ca-central-1",
            "sa-east-1",
        ]
    );
    assert_eq!(
        CROSS_REGION_PREFIXES,
        ["global", "us", "eu", "apac", "jp", "au", "us-gov"]
    );
}

fn routing_prefix_order_case() {
    assert_eq!(
        strip_bedrock_routing_prefix("bedrock/nova-2/model"),
        "model"
    );
    assert_eq!(
        strip_bedrock_routing_prefix("nova/bedrock/model"),
        "bedrock/model",
        "a prefix revealed only after stripping a later one is not retried"
    );
    assert_eq!(strip_bedrock_routing_prefix("plain-model"), "plain-model");
}

fn arn_and_region_path_case() {
    assert_eq!(
        extract_model_from_arn("arn:aws:bedrock:us-east-1:111:foo/bar"),
        "bar"
    );
    assert_eq!(
        extract_model_from_arn("learning/x/y"),
        "y",
        "the substring test matches ordinary words containing arn"
    );
    assert_eq!(extract_model_from_arn("plain/x/y"), "plain/x/y");

    assert_eq!(
        split_bedrock_region_path("bedrock/us-east-1/model-id"),
        (Some("us-east-1".to_owned()), "model-id".to_owned())
    );
    assert_eq!(
        split_bedrock_region_path("bedrock/not-a-region/model-id"),
        (None, "not-a-region/model-id".to_owned()),
        "an unrecognized region-shaped prefix is preserved on the returned id"
    );
}

fn suffix_rewrite_order_case() {
    assert_eq!(
        strip_throughput_suffix("anthropic.claude-3-5-sonnet-20241022-v2:0:51k"),
        "anthropic.claude-3-5-sonnet-20241022-v2:0",
        "the version suffix :0 survives the throughput strip"
    );
    assert_eq!(
        strip_throughput_suffix("us.anthropic.claude-opus-4-6-v1[1m]"),
        "us.anthropic.claude-opus-4-6-v1"
    );
    assert_eq!(
        strip_throughput_suffix("anthropic.claude:1:20k:2:30k"),
        "anthropic.claude:1:20k:2",
        "only the trailing throughput suffix is removed; an earlier one is not re-scanned"
    );
    assert_eq!(
        strip_throughput_suffix("anthropic.claude-3-5-sonnet-20241022-v2:0:51k[1m]"),
        "anthropic.claude-3-5-sonnet-20241022-v2:0:51k",
        "a throughput suffix before a context suffix survives because the throughput strip does not run again"
    );
    assert_eq!(
        strip_throughput_suffix("anthropic.claude-v2:0:51k\n"),
        "anthropic.claude-v2:0\n",
        "Python's unanchored $ also matches just before a trailing newline"
    );
    assert_eq!(
        strip_throughput_suffix("us.anthropic.claude-opus-4-6-v1[1m]\n"),
        "us.anthropic.claude-opus-4-6-v1\n"
    );
}

fn base_model_nova_and_regions_case() {
    assert_eq!(
        get_bedrock_base_model("bedrock/nova-2/arn:aws:sagemaker:us-east-1:1:endpoint/x"),
        "amazon.nova-2-custom"
    );
    assert_eq!(
        get_bedrock_base_model("converse/nova/anything-after-is-ignored"),
        "amazon.nova-custom"
    );
    assert_eq!(
        get_bedrock_base_model("us.meta.llama3-2-11b-instruct-v1:0"),
        "meta.llama3-2-11b-instruct-v1:0",
        "the cross-region prefix is removed and the version suffix is preserved"
    );
    assert_eq!(
        get_bedrock_base_model("bedrock/us-gov-west-1/openai.gpt-oss-20b-1:0"),
        "openai.gpt-oss-20b-1:0",
        "the full-region prefix is removed after the routing prefix"
    );
    assert_eq!(
        get_bedrock_base_model("global"),
        "global",
        "a bare cross-region prefix with no model id is returned unchanged instead of indexing past the end"
    );
}

static BINDER_CALLS: AtomicUsize = AtomicUsize::new(0);
static FAILING_LOAD_CALLS: AtomicUsize = AtomicUsize::new(0);

fn marker() -> usize {
    BINDER_CALLS.fetch_add(1, Ordering::SeqCst);
    42
}

fn lazy_binding_is_cached() {
    let binder: fn() -> usize = marker;
    let mut binding = LazyBinding::from(binder);
    let bound = binding.get();
    let bound_again = binding.get();
    assert_eq!(
        bound, bound_again,
        "the same bound callable is returned on every call"
    );
    assert_eq!(
        BINDER_CALLS.load(Ordering::SeqCst),
        1,
        "the import is cached, not repeated"
    );
}

fn cost_map_success_is_cached() {
    let loaded = cached_catalog(&LOCAL_MODEL_COST_MAP, BundledCatalog::default().asset)
        .expect("the packaged map parses");
    let cached = cached_catalog(&LOCAL_MODEL_COST_MAP, BundledCatalog::default().asset)
        .expect("the cached map parses");
    assert_eq!(
        loaded.entries.len(),
        4458,
        "the real packaged catalog is returned, not a fabricated map"
    );
    assert!(Arc::ptr_eq(&loaded, &cached), "a successful load is cached");
}

fn failing_asset() -> Result<Vec<u8>, CatalogError> {
    FAILING_LOAD_CALLS.fetch_add(1, Ordering::SeqCst);
    Err(CatalogError::Asset(io::Error::other("forced")))
}

fn cost_map_failure_is_never_cached() {
    let cache: Mutex<Option<Arc<ModelCatalog>>> = Mutex::new(None);
    let first = cached_catalog(&cache, failing_asset);
    let second = cached_catalog(&cache, failing_asset);
    assert!(first.is_err(), "a load error is returned, not swallowed");
    assert!(
        second.is_err(),
        "the second call also sees the error, not a cached fallback"
    );
    assert_eq!(
        FAILING_LOAD_CALLS.load(Ordering::SeqCst),
        2,
        "an error is never cached, so the loader runs again on the production caching path"
    );
}

fn lazy_catalog_binding_case() {
    lazy_binding_is_cached();
    cost_map_success_is_cached();
    cost_map_failure_is_never_cached();
}

fn runtime_arn_resource_encoding_case() {
    assert_eq!(
        encode_runtime_modelid_arn("/model/plain-model/invoke"),
        "/model/plain-model/invoke",
        "a non-arn endpoint is returned unchanged"
    );
    assert_eq!(
        encode_runtime_modelid_arn(
            "arn:aws:bedrock:ap-southeast-1:123456789012:application-inference-profile/abdefg12334"
        ),
        "arn:aws:bedrock:ap-southeast-1:123456789012:application-inference-profile%2Fabdefg12334"
    );
    assert_eq!(
        encode_runtime_modelid_arn("arn:aws:bedrock:us-east-1:1:custom-model/foo.bar-1/abc123"),
        "arn:aws:bedrock:us-east-1:1:custom-model%2Ffoo.bar-1%2Fabc123"
    );
    assert_eq!(
        encode_runtime_modelid_arn("arn:aws:bedrock:us-east-1:1:foundation-model/a:prompt/b"),
        "arn:aws:bedrock:us-east-1:1:foundation-model%2Fa:prompt/b",
        "only the first matching resource pattern is replaced"
    );
    assert_eq!(
        encode_runtime_modelid_arn("arn:aws:bedrock:us-east-1:1:unknown-resource/abc"),
        "arn:aws:bedrock:us-east-1:1:unknown-resource/abc",
        "an unrecognized resource type is left unencoded"
    );
}

#[rstest]
#[case::region_groups(region_groups_case)]
#[case::routing_prefix_order(routing_prefix_order_case)]
#[case::arn_and_region_path(arn_and_region_path_case)]
#[case::suffix_rewrite_order(suffix_rewrite_order_case)]
#[case::base_model_nova_and_regions(base_model_nova_and_regions_case)]
#[case::lazy_catalog_binding(lazy_catalog_binding_case)]
#[case::runtime_arn_resource_encoding(runtime_arn_resource_encoding_case)]
fn bedrock_model_cases(#[case] check: fn()) {
    check();
}
