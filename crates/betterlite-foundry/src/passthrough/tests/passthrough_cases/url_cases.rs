use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;

use crate::passthrough::error::FoundryPassthroughError;
use crate::passthrough::tests::{build_plan, object, target};
use crate::passthrough::url::{foundry_root, relay_query_params, without_repeated_native_prefix};

use super::RES;

pub(super) fn foundry_root_path() {
    foundry_root_truncates_at_the_first_exact_models_segment();
    foundry_root_requotes_reserved_characters();
    foundry_root_rejects_a_control_character();
}

fn foundry_root_truncates_at_the_first_exact_models_segment() {
    assert_eq!(
        foundry_root(&format!(
            "{RES}/api/projects/p/models/gpt-4o?api-version=2024#frag"
        ))
        .unwrap(),
        format!("{RES}/api/projects/p#frag")
    );
    assert_eq!(
        foundry_root(&format!("{RES}//api///models//x/")).unwrap(),
        format!("{RES}/api")
    );
    assert_eq!(
        foundry_root(&format!("{RES}/Models/Api/models/x")).unwrap(),
        format!("{RES}/Models/Api")
    );
    assert_eq!(
        foundry_root(&format!("{RES}/one/two/?q=1")).unwrap(),
        format!("{RES}/one/two")
    );
    assert_eq!(
        foundry_root(&format!("{RES}/a%2Fb/models")).unwrap(),
        format!("{RES}/a/b")
    );
    assert_eq!(foundry_root(&format!("{RES}/models")).unwrap(), RES);
    assert_eq!(
        foundry_root("https://example.test").unwrap(),
        "https://example.test"
    );
}

// httpx's copy_with quotes a decoded '?' or '#' back into the path
fn foundry_root_requotes_reserved_characters() {
    assert_eq!(
        foundry_root(&format!("{RES}/a%3Fb/models")).unwrap(),
        format!("{RES}/a%3Fb")
    );
    assert_eq!(
        foundry_root(&format!("{RES}/a%23b/models/x")).unwrap(),
        format!("{RES}/a%23b")
    );
}

fn foundry_root_rejects_a_control_character() {
    assert!(matches!(
        foundry_root("https://example.test/a\nb"),
        Err(FoundryPassthroughError::InvalidApiBase(_))
    ));
}

pub(super) fn repeated_native_prefix() {
    without_repeated_native_prefix_keeps_or_drops_the_overlap();
    complete_urls_strip_the_model_and_model_group();
}

fn without_repeated_native_prefix_keeps_or_drops_the_overlap() {
    let root = format!("{RES}/api/projects/p");
    assert_eq!(
        without_repeated_native_prefix(&root, "api/projects/p/chat/completions").unwrap(),
        root
    );
    assert_eq!(
        without_repeated_native_prefix(&root, "API/Projects/P").unwrap(),
        RES
    );
    assert_eq!(
        without_repeated_native_prefix(&root, "/api/projects/p").unwrap(),
        RES,
        "a leading slash repeats the same path segments"
    );
    assert_eq!(
        without_repeated_native_prefix(&format!("{RES}/V1/OpenAI"), "openai/embeddings").unwrap(),
        format!("{RES}/V1")
    );
    assert_eq!(
        without_repeated_native_prefix(&root, "providers/blackforestlabs/v1/flux-2-pro").unwrap(),
        root
    );
    assert_eq!(
        without_repeated_native_prefix(&root, "").unwrap(),
        root,
        "an empty native endpoint repeats nothing"
    );
}

fn complete_urls_strip_the_model_and_model_group() {
    let litellm_params = object(r#"{"litellm_metadata": {"model_group": "group-one"}}"#);
    let base = format!("{RES}/api/projects/p/models/gpt-x?api-version=2024-01");
    let plan = build_plan(None, None, Some(base.as_str()), None, litellm_params);
    for endpoint in [
        "gpt-x/providers/blackforestlabs/v1/flux-2-pro",
        "group-one/providers/blackforestlabs/v1/flux-2-pro",
    ] {
        let request = PassthroughUrlRequest {
            api_base: None,
            api_key: None,
            target: target("gpt-x", endpoint),
            request_query_params: None,
            litellm_params: &plan.litellm_params,
        };
        let (url, root) = plan.get_complete_url(&request).unwrap();
        assert_eq!(root, format!("{RES}/api/projects/p"));
        assert_eq!(
            url.as_str(),
            format!(
                "{RES}/api/projects/p/providers/blackforestlabs/v1/flux-2-pro?api-version=2024-01"
            )
        );
    }
}

pub(super) fn query_api_version_precedence() {
    caller_versions_win();
    deployment_versions_beat_the_base();
    the_base_version_is_the_fallback();
    complete_urls_read_a_string_deployment_version();
    complete_urls_ignore_a_typed_deployment_version();
}

fn caller_versions_win() {
    let base = format!("{RES}/models?api-version=2024-02&foo=1");
    let caller = object(r#"{"api-version": null, "a": "1"}"#);
    assert_eq!(
        relay_query_params(Some(&caller), Some("2024-06"), &base).unwrap(),
        Some(caller.clone())
    );
    assert_eq!(
        relay_query_params(Some(&object(r#"{"api-version": ""}"#)), Some("2024"), &base).unwrap(),
        Some(object(r#"{"api-version": ""}"#))
    );
    assert_eq!(
        relay_query_params(Some(&caller), Some("2024-06"), "not a url").unwrap(),
        Some(caller),
        "the caller version is read before the base is parsed"
    );
}

fn deployment_versions_beat_the_base() {
    let base = format!("{RES}/models?api-version=2024-02&foo=1");
    assert_eq!(
        relay_query_params(Some(&object(r#"{"a": "1"}"#)), Some("2024-06"), &base).unwrap(),
        Some(object(r#"{"a": "1", "api-version": "2024-06"}"#))
    );
    assert_eq!(
        relay_query_params(Some(&object(r#"{"a": "1"}"#)), Some(""), &base).unwrap(),
        Some(object(r#"{"a": "1", "api-version": "2024-02"}"#))
    );
}

fn the_base_version_is_the_fallback() {
    let base_without_query = format!("{RES}/models");
    assert_eq!(
        relay_query_params(Some(&object(r#"{"a": "1"}"#)), None, &base_without_query).unwrap(),
        Some(object(r#"{"a": "1"}"#))
    );
    assert_eq!(
        relay_query_params(Some(&object("{}")), None, &base_without_query).unwrap(),
        Some(object("{}"))
    );
    assert_eq!(
        relay_query_params(None, None, &base_without_query).unwrap(),
        None
    );
    let base = format!("{RES}/models?api-version=2024-02&foo=1");
    assert_eq!(
        relay_query_params(None, None, &base).unwrap(),
        Some(object(r#"{"api-version": "2024-02"}"#))
    );
    assert_eq!(
        relay_query_params(None, Some("0"), &base).unwrap(),
        Some(object(r#"{"api-version": "0"}"#))
    );
}

fn complete_urls_read_a_string_deployment_version() {
    let base = format!("{RES}/models?api-version=2024-02&foo=1");
    let plan = build_plan(
        None,
        None,
        Some(base.as_str()),
        None,
        object(r#"{"api_version": "2024-06"}"#),
    );
    let request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: target("gpt-x", "gpt-x/chat/completions"),
        request_query_params: None,
        litellm_params: &plan.litellm_params,
    };
    let (url, root) = plan.get_complete_url(&request).unwrap();
    assert_eq!(root, RES);
    assert_eq!(
        url.as_str(),
        format!("{RES}/chat/completions?api-version=2024-06")
    );
}

fn complete_urls_ignore_a_typed_deployment_version() {
    let base = format!("{RES}/models?api-version=2024-02&foo=1");
    for litellm_params in [
        r#"{"api_version": 5}"#,
        r#"{"api_version": null}"#,
        r#"{"api_version": false}"#,
        "{}",
    ] {
        let plan = build_plan(
            None,
            None,
            Some(base.as_str()),
            None,
            object(litellm_params),
        );
        let request = PassthroughUrlRequest {
            api_base: None,
            api_key: None,
            target: target("gpt-x", "gpt-x/chat/completions"),
            request_query_params: None,
            litellm_params: &plan.litellm_params,
        };
        let (url, root) = plan.get_complete_url(&request).unwrap();
        assert_eq!(root, RES, "{litellm_params}");
        assert_eq!(
            url.as_str(),
            format!("{RES}/chat/completions?api-version=2024-02"),
            "{litellm_params}"
        );
    }
}
