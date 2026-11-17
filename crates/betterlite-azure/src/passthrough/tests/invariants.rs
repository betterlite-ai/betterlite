use secrecy::ExposeSecret as _;

use crate::credentials::cache::RefreshProviderCache;
use crate::passthrough::config::{
    AzurePassthroughConfig, AzurePassthroughEnvironment, validate_environment,
};
use crate::passthrough::logging::{NoChatReplyProjection, NoChatStreamAssembly};

use super::support::{
    NoSources, RecordingSources, base_config, complete, deployment, headers, object,
    provider_token, secret, target,
};

#[test]
fn azure_passthrough_credential_identity_isolation() {
    keys_and_bases_stay_per_instance();
    headers_carry_each_instance_key();
    refresh_caches_are_per_instance();
}

#[test]
fn azure_passthrough_secret_safe_failure() {
    configured_secrets_do_not_render_in_debug();
    failed_lookups_name_no_secret();
    the_key_never_reaches_the_url();
}

fn instance(api_base: &str, api_key: &str) -> AzurePassthroughConfig {
    AzurePassthroughConfig {
        api_base_env: Some(api_base.to_owned()),
        api_key_env: Some(secret(api_key)),
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    }
}

fn keys_and_bases_stay_per_instance() {
    let left = instance("https://left.test", "left-key");
    let right = instance("https://right.test", "right-key");
    assert_eq!(left.get_api_base(None), Some("https://left.test"));
    assert_eq!(right.get_api_base(None), Some("https://right.test"));
    assert_eq!(
        left.get_api_key(None)
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("left-key"))
    );
    assert_eq!(
        right
            .get_api_key(None)
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("right-key"))
    );
    let left_url = complete(
        &left,
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap()
    .0;
    let right_url = complete(
        &right,
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap()
    .0;
    assert_eq!(left_url.as_str(), "https://left.test/chat/completions");
    assert_eq!(right_url.as_str(), "https://right.test/chat/completions");
}

fn headers_carry_each_instance_key() {
    let left = instance("https://left.test", "left-key");
    let right = instance("https://right.test", "right-key");
    let credentials = deployment();
    let sources = NoSources;
    let left_headers = validate_environment(AzurePassthroughEnvironment {
        headers: headers(&[("x-identity", "left")]),
        api_key: left.get_api_key(None),
        params_key: None,
        configured_key: None,
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        left_headers.get("api-key").map(String::as_str),
        Some("left-key")
    );
    assert_eq!(
        left_headers.get("x-identity").map(String::as_str),
        Some("left")
    );
    let right_headers = validate_environment(AzurePassthroughEnvironment {
        headers: headers(&[]),
        api_key: right.get_api_key(None),
        params_key: None,
        configured_key: None,
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        right_headers.get("api-key").map(String::as_str),
        Some("right-key")
    );
}

fn refresh_caches_are_per_instance() {
    let left_sources = RecordingSources::new("left");
    let mut left_cache = RefreshProviderCache::new(&left_sources);
    let provider = left_cache.provider("scope-a").unwrap();
    assert_eq!(provider_token(provider.as_ref()), "left:scope-a");
    let cached = left_cache.provider("scope-a").unwrap();
    assert_eq!(provider_token(cached.as_ref()), "left:scope-a");
    assert_eq!(left_sources.calls(), vec![String::from("left:scope-a")]);
    let right_sources = RecordingSources::new("right");
    let mut right_cache = RefreshProviderCache::new(&right_sources);
    let right_provider = right_cache.provider("scope-a").unwrap();
    assert_eq!(provider_token(right_provider.as_ref()), "right:scope-a");
    assert_eq!(right_sources.calls(), vec![String::from("right:scope-a")]);
    let other = right_cache.provider("scope-b").unwrap();
    assert_eq!(provider_token(other.as_ref()), "right:scope-b");
    assert_eq!(
        right_sources.calls(),
        vec![String::from("right:scope-a"), String::from("right:scope-b")]
    );
}

fn configured_secrets_do_not_render_in_debug() {
    let config = instance("https://x.test", "hunter2-azure-key");
    let rendered = format!("{config:?}");
    assert!(rendered.contains("https://x.test"));
    assert!(!rendered.contains("hunter2-azure-key"));
}

fn failed_lookups_name_no_secret() {
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: Some(secret("hunter2-azure-key")),
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    };
    let error = complete(
        &config,
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Azure api base not found");
    assert!(!format!("{error:?}").contains("hunter2-azure-key"));
    let query = object(r#"{"api-version": {"nested": "hunter2-azure-key"}}"#);
    let query_error = complete(
        &base_config(None),
        Some("https://x.test"),
        target("m", "chat/completions"),
        Some(&query),
        &object("{}"),
    )
    .unwrap_err();
    assert_eq!(
        query_error.to_string(),
        "the query parameter api-version is not a string or a number"
    );
    assert!(!format!("{query_error:?}").contains("hunter2-azure-key"));
}

fn the_key_never_reaches_the_url() {
    let config = instance("https://x.test", "hunter2-azure-key");
    let url = complete(
        &config,
        None,
        target("m", "chat/completions"),
        None,
        &object("{}"),
    )
    .unwrap()
    .0;
    assert_eq!(url.as_str(), "https://x.test/chat/completions");
    assert!(!url.as_str().contains("hunter2-azure-key"));
}
