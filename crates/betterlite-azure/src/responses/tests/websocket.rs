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
}
