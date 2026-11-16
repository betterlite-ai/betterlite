use crate::azure_urls::AzureUrlError;
use crate::responses::error::ResponsesError;
use crate::responses::url::{
    MODEL_IN_WEBSOCKET_URL, SUPPORTS_NATIVE_WEBSOCKET, WEBSOCKET_ROUTE, websocket_url,
};

fn rewritten(api_base: &str) -> String {
    websocket_url(Some(api_base)).unwrap()
}

pub(super) fn websocket_model_query() {
    assert_eq!(
        [SUPPORTS_NATIVE_WEBSOCKET, MODEL_IN_WEBSOCKET_URL],
        [true, false]
    );
    assert_eq!(WEBSOCKET_ROUTE, "/openai/v1/responses");
    assert!(matches!(
        websocket_url(None),
        Err(ResponsesError::MissingWebsocketApiBase)
    ));
    // a trailing slash and an existing responses route are replaced by the websocket route
    let resource = "wss://res.openai.azure.com/openai/v1/responses";
    assert_eq!(rewritten("https://res.openai.azure.com"), resource);
    assert_eq!(rewritten("https://res.openai.azure.com/"), resource);
    assert_eq!(
        rewritten("https://res.openai.azure.com/openai/responses?api-version=2024-01-01"),
        resource
    );
    assert_eq!(
        rewritten("https://h/base/openai/v1/responses"),
        "wss://h/base/openai/v1/responses"
    );
    // every query parameter goes, the fragment stays
    assert_eq!(
        rewritten("https://h/deployments/d?x=1#y"),
        "wss://h/deployments/d/openai/v1/responses#y"
    );
    websocket_schemes();
    websocket_paths();
}

fn websocket_schemes() {
    // only an https base switches to wss; anything else stays ws
    assert_eq!(
        rewritten("http://host:8080/foo/"),
        "ws://host:8080/foo/openai/v1/responses"
    );
    assert_eq!(
        rewritten("wss://host/base"),
        "ws://host/base/openai/v1/responses"
    );
}

fn websocket_paths() {
    // httpx decodes the path before the suffix check and the new route is quoted again
    assert_eq!(
        rewritten("https://h/a%2Fb"),
        "wss://h/a/b/openai/v1/responses"
    );
    assert_eq!(
        rewritten("https://h/openai%2Fresponses"),
        "wss://h/openai/v1/responses"
    );
    // one decode only: an escaped percent stays quoted for httpx's path
    assert_eq!(
        rewritten("http://h/a%252Fb"),
        "ws://h/a%2Fb/openai/v1/responses"
    );
    // a decoded '?' or '#' cannot be written back as a path component
    assert!(matches!(
        websocket_url(Some("https://h/a%3Fb")),
        Err(ResponsesError::Url(AzureUrlError::UnrewritablePath))
    ));
    assert!(matches!(
        websocket_url(Some("https://h/a%23b")),
        Err(ResponsesError::Url(AzureUrlError::UnrewritablePath))
    ));
}
