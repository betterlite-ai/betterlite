use std::time::Duration;

use betterlite_prompts::arguments::json::Object;

use crate::embeddings::error::FoundryEmbeddingError;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;

use super::support::{
    Answer, BASE, Factory, IMAGE_ONE, IMAGE_URI, ImageClient, MODEL, Seen, TEXT_EMPTY, TEXT_ONE,
    Text, key, object, request, run, strings,
};

pub(super) fn inline_image_embedding() {
    sync_reuses_supplied();
    sync_replaces_client();
    async_reuses_supplied();
    async_mismatched_supplied();
    images_before_text();
}

fn sync_reuses_supplied() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let supplied = ImageClient {
        answer: Answer::json(IMAGE_ONE),
        seen: Seen::default(),
    };
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_EMPTY);
    let held = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied,
    };
    let response = run(&call, Some(held), &factory, &text).unwrap();
    assert_eq!(response.data.len(), 1);
    assert!(factory.requests().is_empty());
    assert!(factory.sync_timeouts().is_empty());
    let sent = supplied.seen.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, http::Method::POST);
    assert_eq!(sent[0].url, format!("{BASE}/models/images/embeddings"));
    assert_eq!(
        sent[0].headers.get("authorization").unwrap(),
        "Bearer case-key"
    );
    assert_eq!(
        sent[0].headers.get("content-type").unwrap(),
        "application/json"
    );
    let body: Object = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(
        body,
        object(&format!(r#"{{"input":[{{"image":"{IMAGE_URI}"}}]}}"#))
    );
}

fn sync_replaces_client() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    run(&call, None, &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    assert_eq!(factory.sync_timeouts(), vec![Duration::from_secs(5)]);
    assert!(factory.async_timeouts().is_empty());
    assert_eq!(factory.requests().len(), 1);
}

fn async_reuses_supplied() {
    let (supplied, factory) = async_with_supplied(ClientKind::Asynchronous);
    assert_eq!(supplied.seen.lock().unwrap().len(), 1);
    assert!(factory.requests().is_empty());
    assert!(factory.async_timeouts().is_empty());
}

fn async_mismatched_supplied() {
    let (supplied, factory) = async_with_supplied(ClientKind::Sync);
    assert!(supplied.seen.lock().unwrap().is_empty());
    assert_eq!(factory.async_timeouts(), vec![Duration::from_secs(5)]);
    assert_eq!(factory.requests().len(), 1);
}

fn async_with_supplied(held_mode: ClientKind) -> (ImageClient, Factory) {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.mode = ClientKind::Asynchronous;
    call.api_key = Some(&secret);
    let supplied = ImageClient {
        answer: Answer::json(IMAGE_ONE),
        seen: Seen::default(),
    };
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let held = SuppliedClient {
        kind: held_mode,
        transport: &supplied,
    };
    run(&call, Some(held), &factory, &Text::replying(TEXT_EMPTY)).unwrap();
    (supplied, factory)
}

fn images_before_text() {
    let input = strings(&[IMAGE_URI, "hello"]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let null_data = r#"{"object":"list","data":null,"model":"image-model"}"#;
    let factory = Factory::new(Answer::json(null_data));
    let text = Text::replying(TEXT_ONE);
    let sync_error = run(&call, None, &factory, &text).unwrap_err();
    assert!(matches!(
        sync_error,
        FoundryEmbeddingError::ImageDataMissing
    ));
    assert!(text.calls().is_empty());
    call.mode = ClientKind::Asynchronous;
    let async_error = run(&call, None, &factory, &text).unwrap_err();
    assert!(matches!(
        async_error,
        FoundryEmbeddingError::ImageDataMissing
    ));
    assert!(text.calls().is_empty());
    assert_eq!(factory.requests().len(), 2);
}
