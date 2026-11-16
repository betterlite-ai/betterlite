use crate::chat::url::get_complete_url;
use crate::chat::url_error::UrlError;

const REQUIRED_BASE: &str = "api_base is required for Azure AI Studio. Please set the api_base parameter. Passed `api_base=None`";

fn url(base: &str, api_version: Option<&str>) -> String {
    get_complete_url(Some(base), api_version).expect("the fixture base composes")
}

pub(super) fn models_chat_completions_route() {
    let missing = get_complete_url(None, None).expect_err("a missing base is refused");
    assert_eq!(missing.to_string(), REQUIRED_BASE);
    assert!(matches!(missing, UrlError::MissingApiBase));

    assert_eq!(
        url("https://litellm8397336933.services.ai.azure.com", None),
        "https://litellm8397336933.services.ai.azure.com/models/chat/completions"
    );
    assert_eq!(
        url("https://studio.example.test", None),
        "https://studio.example.test/chat/completions"
    );
    // the raw substring decides, so a base B549 would not recognise as Foundry still takes the models route
    assert_eq!(
        url("https://x.services.ai.azure.com.evil.test", None),
        "https://x.services.ai.azure.com.evil.test/models/chat/completions"
    );
    assert_eq!(
        url(
            "https://proxy.example.test/services.ai.azure.com/route",
            None
        ),
        "https://proxy.example.test/services.ai.azure.com/route/models/chat/completions"
    );
}

pub(super) fn api_version_url_precedence() {
    assert_eq!(
        url(
            "https://studio.example.test/v1?api-version=2024-05-01-preview",
            Some("ignored")
        ),
        "https://studio.example.test/v1/chat/completions?api-version=2024-05-01-preview"
    );
    // the existing api-version wins even when other query names are present too
    assert_eq!(
        url(
            "https://studio.example.test/v1?api-version=given&a=1",
            Some("configured")
        ),
        "https://studio.example.test/v1/chat/completions?api-version=given&a=1"
    );
    // an empty existing api-version still blocks the configured one
    assert_eq!(
        url("https://studio.example.test/v1?api-version=", Some("v")),
        "https://studio.example.test/v1/chat/completions?api-version="
    );
    assert_eq!(
        url("https://studio.example.test/v1?a=1&b=2", Some("v")),
        "https://studio.example.test/v1/chat/completions?a=1&b=2&api-version=v"
    );
    assert_eq!(
        url("https://studio.example.test/v1?a=1&a=2", None),
        "https://studio.example.test/v1/chat/completions?a=1"
    );
    assert_eq!(
        url("https://studio.example.test/v1", Some("")),
        "https://studio.example.test/v1/chat/completions"
    );
}

#[test]
fn url_keeps_overlapping_route_segments() {
    assert_eq!(
        url("https://x.services.ai.azure.com/models", None),
        "https://x.services.ai.azure.com/models/chat/completions"
    );
    assert_eq!(
        url("https://studio.example.test/v1/chat/completions", Some("v")),
        "https://studio.example.test/v1/chat/completions?api-version=v"
    );
    assert_eq!(
        url("https://studio.example.test/", Some("v")),
        "https://studio.example.test/chat/completions?api-version=v"
    );
}

#[test]
fn url_reencodes_query_like_httpx() {
    assert_eq!(
        url("https://studio.example.test/v1?x=hello world", None),
        "https://studio.example.test/v1/chat/completions?x=hello+world"
    );
    assert_eq!(
        url("https://studio.example.test/v1?x=a*b~", None),
        "https://studio.example.test/v1/chat/completions?x=a%2Ab~"
    );
    assert_eq!(
        url("https://studio.example.test/v1?z=1#frag", Some("v")),
        "https://studio.example.test/v1/chat/completions?z=1&api-version=v#frag"
    );
    assert_eq!(
        url("https://studio.example.test/v1?", None),
        "https://studio.example.test/v1/chat/completions"
    );
    assert_eq!(
        url("https://studio.example.test/v1?a", None),
        "https://studio.example.test/v1/chat/completions?a="
    );
}

#[test]
fn url_refuses_an_unparseable_base() {
    let refused = get_complete_url(Some("not a url"), None).expect_err("the base is not a URL");
    assert!(matches!(refused, UrlError::InvalidUrl(_)));
}
