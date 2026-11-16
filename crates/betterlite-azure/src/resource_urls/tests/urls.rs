use betterlite_http::user_url::error::UrlParseError;
use betterlite_openai::videos::create::VideoCreate;

use crate::azure_urls::AzureUrlError;
use crate::resource_urls::error::ResourceUrlError;
use crate::resource_urls::{
    AzureResource, UrlRequest, VIDEO_SUPPORTED_PARAMS, complete_url, container_api_version,
    container_resource_root, video_map_params,
};

use super::cases::{
    CONTAINER_BASE_CASES, CONTAINER_QUERY_CASES, MODULE_BASE, UrlCase, VECTOR_CASES,
};

pub(super) fn expect_urls(resource: AzureResource, cases: &[UrlCase]) {
    for (api_base, api_version, expected) in cases {
        let request = UrlRequest {
            api_base: *api_base,
            fallback_api_base: None,
            api_version: *api_version,
        };
        assert_eq!(
            complete_url(resource, &request).unwrap(),
            *expected,
            "{api_base:?} {api_version:?}"
        );
    }
}

pub(super) fn container_api_version_query() {
    expect_urls(AzureResource::Container, &CONTAINER_QUERY_CASES);
    let encoded = "https://r.example/openai/responses?api-version=%32%30";
    assert_eq!(
        container_api_version(encoded).unwrap(),
        Some("20".to_owned())
    );
    assert_eq!(
        container_api_version("https://r.example/openai/responses").unwrap(),
        None
    );
    let invalid = UrlRequest {
        api_base: Some("http://[::1/openai/responses"),
        fallback_api_base: None,
        api_version: None,
    };
    assert!(matches!(
        complete_url(AzureResource::Container, &invalid).unwrap_err(),
        ResourceUrlError::Parse(UrlParseError::InvalidIpv6Url)
    ));
}

pub(super) fn container_normalized_base() {
    expect_urls(AzureResource::Container, &CONTAINER_BASE_CASES);
    for unchanged in [
        "https://r.example/openai/v1/responses",
        "https://r.example/other?x=1#f",
        "https://r.example/openai/responses/extra",
        "",
    ] {
        assert_eq!(container_resource_root(unchanged).unwrap(), unchanged);
    }
    assert_eq!(
        container_resource_root("HTTPS://R.example/a/openai/responses/?q#f").unwrap(),
        "https://R.example/a"
    );
    container_fallback_base();
}

fn container_fallback_base() {
    // a falsy api_base is left alone, and the module value is read afterwards without the responses suffix cut
    let empty = UrlRequest {
        api_base: Some(""),
        fallback_api_base: Some(MODULE_BASE),
        api_version: None,
    };
    assert_eq!(
        complete_url(AzureResource::Container, &empty).unwrap(),
        "https://m.example/openai/v1/responses/openai/v1/containers?api-version=2025-04-01-preview"
    );
    let absent = UrlRequest {
        api_base: None,
        fallback_api_base: Some(MODULE_BASE),
        api_version: Some("2025-01-01"),
    };
    assert_eq!(
        complete_url(AzureResource::Container, &absent).unwrap(),
        "https://m.example/openai/responses/openai/containers?api-version=2025-04-01-preview"
    );
    let missing = UrlRequest {
        api_base: None,
        fallback_api_base: None,
        api_version: None,
    };
    assert!(matches!(
        complete_url(AzureResource::Container, &missing).unwrap_err(),
        ResourceUrlError::Url(AzureUrlError::MissingApiBase)
    ));
}

pub(super) fn video_supported_fields() {
    assert_eq!(
        VIDEO_SUPPORTED_PARAMS,
        [
            "model",
            "prompt",
            "input_reference",
            "seconds",
            "size",
            "user",
            "extra_headers"
        ]
    );
    assert!(!VIDEO_SUPPORTED_PARAMS.contains(&"characters"));
    for entry in [
        r#"{"seconds":"5"}"#,
        r#"{"size":"720x1280"}"#,
        r#"{"user":"u-1"}"#,
        r#"{"characters":[{"id":"char-1"}]}"#,
        r#"{"input_reference":"ref"}"#,
        r#"{"extra_headers":{"x-one":"1"}}"#,
        r#"{"model":"option-model"}"#,
        r#"{"unsupported":["kept"]}"#,
    ] {
        let options = serde_json::from_str::<VideoCreate>(entry).unwrap();
        assert_eq!(
            serde_json::to_string(&video_map_params(&options)).unwrap(),
            entry
        );
    }
    assert!(video_map_params(&VideoCreate::default()).is_empty());
}

pub(super) fn vector_search_azure_base() {
    expect_urls(AzureResource::VectorStore, &VECTOR_CASES);
    let request = UrlRequest {
        api_base: Some(""),
        fallback_api_base: Some("https://m.example"),
        api_version: None,
    };
    assert_eq!(
        complete_url(AzureResource::VectorStore, &request).unwrap(),
        "https://m.example/openai/vector_stores"
    );
    super::auth::vector_store_headers();
}
