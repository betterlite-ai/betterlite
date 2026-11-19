use super::super::ocr::OcrDocumentKind;
use super::*;

#[tokio::test]
async fn ocr_without_a_config_returns_none() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let text = state
        .ocr(Some(b"content"), Some("text/plain"))
        .await
        .expect("ocr");
    assert!(text.is_none());
    assert!(harness.ocr.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_without_content_returns_none() {
    let harness = Harness::default();
    let parsed = serde_json::from_str(r#"{"ocr": {"model": "mistral/mistral-ocr-latest"}}"#)
        .expect("the options parse");
    let state = harness.state(parsed);
    assert!(
        state
            .ocr(None, Some("text/plain"))
            .await
            .expect("ocr")
            .is_none()
    );
    assert!(
        state
            .ocr(Some(b""), Some("text/plain"))
            .await
            .expect("ocr")
            .is_none()
    );
    assert!(harness.ocr.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_defaults_the_model_and_encodes_a_document_data_url() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let text = state
        .ocr(Some(b"hello"), Some("application/pdf"))
        .await
        .expect("ocr");
    assert_eq!(text.as_deref(), Some("page"));
    let calls = harness.ocr.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "mistral/mistral-ocr-latest");
    assert_eq!(calls[0].document.kind, OcrDocumentKind::DocumentUrl);
    assert_eq!(
        calls[0].document.data_url,
        "data:application/pdf;base64,aGVsbG8="
    );
}

#[tokio::test]
async fn ocr_uses_the_configured_model() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(
        serde_json::from_str(r#"{"ocr": {"model": "mistral/mistral-ocr-2505"}}"#)
            .expect("the options parse"),
    );
    state
        .ocr(Some(b"hello"), Some("text/plain"))
        .await
        .expect("ocr");
    assert_eq!(
        harness.ocr.calls.lock().unwrap()[0].model,
        "mistral/mistral-ocr-2505"
    );
}

#[test]
fn ocr_document_kinds_use_their_litellm_wire_names() {
    assert_eq!(OcrDocumentKind::ImageUrl.wire_name(), "image_url");
    assert_eq!(OcrDocumentKind::DocumentUrl.wire_name(), "document_url");
}

#[tokio::test]
async fn ocr_image_content_types_use_the_image_url_key() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    state
        .ocr(Some(b"img"), Some("image/png"))
        .await
        .expect("ocr");
    let calls = harness.ocr.calls.lock().unwrap();
    assert_eq!(calls[0].document.kind, OcrDocumentKind::ImageUrl);
    assert_eq!(calls[0].document.data_url, "data:image/png;base64,aW1n");
}

#[tokio::test]
async fn ocr_without_a_content_type_uses_an_empty_mime() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec!["page".to_owned()]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    state.ocr(Some(b"x"), None).await.expect("ocr");
    assert_eq!(
        harness.ocr.calls.lock().unwrap()[0].document.data_url,
        "data:;base64,eA=="
    );
}

#[tokio::test]
async fn ocr_prefers_the_router_operation() {
    let harness = Harness::default();
    let routed = Arc::new(FakeOcr::returning(vec!["router".to_owned()]));
    let router: Arc<dyn OcrOperation> = Arc::<FakeOcr>::clone(&routed);
    let state = harness.routed(
        serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"),
        Some(router),
    );
    let text = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect("ocr");
    assert_eq!(text.as_deref(), Some("router"));
    assert_eq!(routed.calls.lock().unwrap().len(), 1);
    assert!(harness.ocr.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ocr_joins_pages_with_a_blank_line() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(vec![
        "first".to_owned(),
        "second".to_owned(),
    ]));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let text = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect("ocr");
    assert_eq!(text.as_deref(), Some("first\n\nsecond"));
}

#[tokio::test]
async fn ocr_without_pages_returns_none() {
    let harness = Harness::default().with_ocr(FakeOcr::returning(Vec::new()));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let text = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect("ocr");
    assert!(text.is_none());
}

#[tokio::test]
async fn ocr_propagates_the_operation_error() {
    let harness = Harness::default().with_ocr(FakeOcr::failing("the provider failed"));
    let state = harness.state(serde_json::from_str(r#"{"ocr": {}}"#).expect("the options parse"));
    let error = state
        .ocr(Some(b"x"), Some("text/plain"))
        .await
        .expect_err("the OCR fails");
    assert_eq!(
        error,
        OcrError::Failed {
            message: "the provider failed".to_owned()
        }
    );
}
