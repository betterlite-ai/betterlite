use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret as _;

use crate::s3_access::decode::decode_managed_file_id;
use crate::s3_access::managed::resolve_managed_s3_object;
use crate::s3_access::models::{invoke_route_model, strip_llm_routing_prefix};
use crate::s3_access::settings::{S3Setting, S3SettingSources, SettingMap, resolve_s3_setting};
use crate::s3_access::{TrustedValue, configured_bucket_name};

use super::fixtures::{MANAGED_FILE_ID, MANAGED_OBJECT};
use super::support::{environment, params, rendered, secrets, settings, snapshot};

#[test]
fn the_shared_functions_serve_extraction_and_bucket_selection() {
    assert_eq!(
        decode_managed_file_id(MANAGED_FILE_ID).expect("the managed id decodes"),
        MANAGED_OBJECT
    );
    let configured = params(snapshot(&[(
        "s3_bucket_name",
        TrustedValue::Text("managed-bucket".to_owned()),
    )]));
    assert_eq!(
        configured_bucket_name(&configured, &environment(&[])).expect("the bucket is configured"),
        "managed-bucket"
    );
    assert_eq!(
        resolve_managed_s3_object(MANAGED_FILE_ID, &configured, &environment(&[]))
            .expect("the managed object belongs to the configured bucket"),
        (
            "managed-bucket".to_owned(),
            "litellm-bedrock-files-file-123.jsonl".to_owned()
        )
    );
}

#[test]
fn replays_the_bedrock_services_s3_access_corpus() {
    let seeds = replay_corpus();
    assert!(seeds >= 4, "the corpus holds only {seeds} seeds");
}

fn replay_corpus() -> usize {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_s3_access");
    let entries = fs::read_dir(corpus).expect("the corpus directory exists");
    let mut seeds = 0_usize;
    for entry in entries {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let value = String::from_utf8(bytes).expect("the seed is UTF-8");
        let expected = name.starts_with("valid") || name.starts_with("boundary-valid");
        assert_eq!(
            decode_managed_file_id(&value).is_ok(),
            expected,
            "seed {name}"
        );
        seeds = seeds.saturating_add(1);
    }
    seeds
}

#[test]
fn bedrock_services_s3_access_credential_identity_isolation() {
    assert_bucket_identity_isolation();
    assert_setting_identity_isolation();
}

fn assert_bucket_identity_isolation() {
    let first = params(snapshot(&[(
        "s3_bucket_name",
        TrustedValue::Text("tenant-a".to_owned()),
    )]));
    let second = params(snapshot(&[(
        "s3_bucket_name",
        TrustedValue::Text("tenant-b".to_owned()),
    )]));
    let no_environment = environment(&[]);
    resolve_managed_s3_object(
        "s3://tenant-a/litellm-bedrock-files-shared",
        &first,
        &no_environment,
    )
    .expect("tenant a owns its bucket");
    resolve_managed_s3_object(
        "s3://tenant-a/litellm-bedrock-files-shared",
        &second,
        &no_environment,
    )
    .expect_err("tenant b does not own tenant a's bucket");
    resolve_managed_s3_object(
        "s3://tenant-b/litellm-bedrock-files-shared",
        &first,
        &no_environment,
    )
    .expect_err("tenant a does not own tenant b's bucket");
    resolve_managed_s3_object(
        "s3://tenant-b/litellm-bedrock-files-shared",
        &second,
        &no_environment,
    )
    .expect("tenant b owns its bucket");
    assert!(!first.trusted.allows_legacy());
}

fn assert_setting_identity_isolation() {
    let first_keys = settings(&[("s3_encryption_key_id", "key-tenant-a")]);
    let second_keys = settings(&[("s3_encryption_key_id", "key-tenant-b")]);
    let lookup = secrets();
    let no_environment = BTreeMap::new();
    assert_eq!(
        resolved_setting(&first_keys, &lookup, &no_environment).as_deref(),
        Some("key-tenant-a")
    );
    assert_eq!(
        resolved_setting(&second_keys, &lookup, &no_environment).as_deref(),
        Some("key-tenant-b")
    );
}

fn resolved_setting(
    configuration: &SettingMap,
    lookup: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    let sources = S3SettingSources {
        params: configuration,
        optional_params: &SettingMap::new(),
        secrets: lookup,
        environment,
    };
    resolve_s3_setting(S3Setting::EncryptionKeyId, &sources)
        .expect("the fixture lookup cannot fail")
        .map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn bedrock_services_s3_access_secret_safe_failure() {
    let bucket_secret = "fixture-secret-bucket";
    let configured = params(snapshot(&[(
        "s3_bucket_name",
        TrustedValue::Text(bucket_secret.to_owned()),
    )]));
    let error = resolve_managed_s3_object(
        "s3://foreign/litellm-bedrock-files-x",
        &configured,
        &environment(&[]),
    )
    .expect_err("the foreign bucket is rejected");
    let rendered_error = rendered(&error);
    assert!(!rendered_error.contains(bucket_secret));

    let key_secret = "fixture-secret-key-id";
    let lookup = secrets();
    let secret_environment = environment(&[("AWS_S3_ENCRYPTION_KEY_ID", key_secret)]);
    let empty = SettingMap::new();
    let sources = S3SettingSources {
        params: &empty,
        optional_params: &SettingMap::new(),
        secrets: &lookup,
        environment: &secret_environment,
    };
    let resolved =
        resolve_s3_setting(S3Setting::EncryptionKeyId, &sources).expect("the lookup cannot fail");
    assert!(resolved.is_some());
    let key_rendered = format!("{resolved:?}");
    assert!(!key_rendered.contains(key_secret));
}

#[test]
fn invoke_and_strip_use_the_shared_resolver() {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let no_environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &no_environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    assert_eq!(
        strip_llm_routing_prefix("bedrock/anthropic.claude-3-sonnet", &context),
        "anthropic.claude-3-sonnet"
    );
    assert_eq!(
        invoke_route_model("bedrock/anthropic.claude-3-sonnet", &context),
        "invoke/anthropic.claude-3-sonnet"
    );
    assert_eq!(invoke_route_model("invoke/foo", &context), "invoke/foo");
}
