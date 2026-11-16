use crate::azure_urls::AzureUrlError;
use crate::responses::error::ResponsesError;
use crate::responses::url::{
    MODEL_IN_WEBSOCKET_URL, SUPPORTS_NATIVE_WEBSOCKET, WEBSOCKET_ROUTE, websocket_url,
};

pub(super) fn websocket_model_query() {
    assert!(SUPPORTS_NATIVE_WEBSOCKET);
    assert!(!MODEL_IN_WEBSOCKET_URL);
    assert_eq!(WEBSOCKET_ROUTE, "/openai/v1/responses");
    assert!(matches!(
        websocket_url(None),
        Err(ResponsesError::MissingWebsocketApiBase)
    ));
    // a trailing slash and an existing responses route are replaced by the websocket route
    assert_eq!(
        websocket_url(Some("https://res.openai.azure.com")).unwrap(),
        "wss://res.openai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        websocket_url(Some("https://res.openai.azure.com/")).unwrap(),
        "wss://res.openai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        websocket_url(Some(
            "https://res.openai.azure.com/openai/responses?api-version=2024-01-01"
        ))
        .unwrap(),
        "wss://res.openai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        websocket_url(Some("https://h/base/openai/v1/responses")).unwrap(),
        "wss://h/base/openai/v1/responses"
    );
    // every query parameter goes, the fragment stays
    assert_eq!(
        websocket_url(Some("https://h/deployments/d?x=1#y")).unwrap(),
        "wss://h/deployments/d/openai/v1/responses#y"
    );
    // only an https base switches to wss; anything else stays ws
    assert_eq!(
        websocket_url(Some("http://host:8080/foo/")).unwrap(),
        "ws://host:8080/foo/openai/v1/responses"
    );
    assert_eq!(
        websocket_url(Some("wss://host/base")).unwrap(),
        "ws://host/base/openai/v1/responses"
    );
    // httpx decodes the path before the suffix check and the new route is quoted again
    assert_eq!(
        websocket_url(Some("https://h/a%2Fb")).unwrap(),
        "wss://h/a/b/openai/v1/responses"
    );
    assert_eq!(
        websocket_url(Some("https://h/openai%2Fresponses")).unwrap(),
        "wss://h/openai/v1/responses"
    );
    // one decode only: an escaped percent stays quoted for httpx's path
    assert_eq!(
        websocket_url(Some("http://h/a%252Fb")).unwrap(),
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
