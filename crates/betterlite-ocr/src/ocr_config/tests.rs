#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::json::{Json, Object};
use crate::ocr_config::connection::{
    OcrConnection, OcrCredentialInputs, OcrInputSource, OcrSourced, OcrTransportConfig,
    ResolvedOcrCredentials,
};
use crate::ocr_config::decode::{compose_body, decode_request_value, decode_response};
use crate::ocr_config::{
    DecodedOcrResponse, OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrRequestContext,
    OcrTransform, Secrets,
};
use crate::ocr_document::{OcrDocument, validate_inline_document};
use crate::ocr_error::OcrError;
use crate::ocr_http::tests::support::{
    TestBody, TestConfig, TestPayload, connection, object, ocr_client,
};
use crate::ocr_settings::OcrSettings;

fn document() -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: "data:application/pdf;base64,YWJj".to_owned(),
        extra: BTreeMap::new(),
    }
}

fn settings_with_request_timeout(request_timeout: Duration) -> OcrSettings {
    OcrSettings {
        request_timeout,
        max_download_bytes: 50 * 1024 * 1024,
        poll_timeout: Duration::from_mins(2),
        doc_intelligence_api_version: "2024-11-30".to_owned(),
        doc_intelligence_dpi: 96,
        vertex_project: None,
        vertex_location: None,
        enable_azure_ad_token_refresh: false,
    }
}

#[test]
fn resolve_connection_params_prefers_the_dynamic_values() {
    let inputs = OcrCredentialInputs {
        api_key: Some(OcrSourced {
            value: SecretString::from("static-key"),
            source: OcrInputSource::Deployment,
        }),
        dynamic_api_key: Some(OcrSourced {
            value: SecretString::from("dynamic-key"),
            source: OcrInputSource::Request,
        }),
        api_base: Some(OcrSourced {
            value: "static-base".to_owned(),
            source: OcrInputSource::Deployment,
        }),
        dynamic_api_base: Some(OcrSourced {
            value: String::new(),
            source: OcrInputSource::Request,
        }),
    };
    let resolved = TestConfig.resolve_connection_params(inputs);
    let key = resolved.api_key.expect("a key is resolved");
    assert_eq!(key.value.expose_secret(), "dynamic-key");
    assert_eq!(key.source, OcrInputSource::Request);
    let base = resolved.api_base.expect("a base is resolved");
    assert_eq!(base.value, "static-base");
    assert_eq!(base.source, OcrInputSource::Deployment);
}

#[test]
fn connection_inputs_drop_blank_values() {
    let inputs = OcrCredentialInputs::new(
        Some(SecretString::from(" key ")),
        OcrInputSource::Request,
        Some("   ".to_owned()),
        OcrInputSource::Environment,
    );
    let key = inputs.api_key.expect("the key survives");
    assert_eq!(key.value.expose_secret(), "key");
    assert_eq!(key.source, OcrInputSource::Request);
    assert!(inputs.api_base.is_none());
}

#[test]
fn connection_timeout_falls_back_to_the_request_timeout_setting() {
    let settings = settings_with_request_timeout(Duration::from_secs(42));
    let timeout = |call_timeout: Option<Duration>| {
        OcrConnection::new(
            ResolvedOcrCredentials::default(),
            OcrTransportConfig {
                extra_headers: Vec::new(),
                extra_headers_source: OcrInputSource::Deployment,
                timeout: call_timeout,
                max_response_bytes: OCR_RESPONSE_MAX_BYTES,
            },
            settings.clone(),
            Secrets::new(),
        )
        .timeout
    };
    assert_eq!(timeout(None), Duration::from_secs(42));
    assert_eq!(timeout(Some(Duration::ZERO)), Duration::from_secs(42));
    assert_eq!(
        timeout(Some(Duration::from_secs(5))),
        Duration::from_secs(5)
    );
}

#[test]
fn health_check_document_is_the_pdf_data_uri() {
    let OcrDocument::DocumentUrl {
        document_url,
        extra,
    } = TestConfig.get_health_check_document()
    else {
        panic!("the health check document is a document_url");
    };
    assert!(document_url.starts_with("data:application/pdf;base64,"));
    assert!(extra.is_empty());
    validate_inline_document(&OcrDocument::DocumentUrl {
        document_url,
        extra: BTreeMap::new(),
    })
    .expect("the health check document is a valid data URI");
}

#[test]
fn validate_request_body_accepts_any_body_by_default() {
    TestConfig
        .validate_request_body(&Json::Object(Object::new()))
        .expect("an empty body is accepted");
    TestConfig
        .validate_request_body(&Json::Text("anything".to_owned()))
        .expect("any other body is accepted");
}

#[tokio::test]
async fn async_transform_ocr_request_defaults_to_the_sync_transform() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let connection = connection(OcrSettings::default());
    let params = ();
    let headers = vec![("authorization".to_owned(), "Bearer x".to_owned())];
    let transform = OcrTransform {
        model: "requested-model",
        optional_params: &params,
        headers: &headers,
    };
    let document = document();
    let context = OcrRequestContext {
        client: &client,
        connection: &connection,
    };
    let asynchronous = TestConfig
        .async_transform_ocr_request(document.clone(), transform, context)
        .await
        .expect("the default transform succeeds");
    let synchronous = TestConfig
        .transform_ocr_request(document, transform)
        .expect("the sync transform succeeds");
    assert_eq!(asynchronous.model, synchronous.model);
    assert_eq!(asynchronous.document, synchronous.document);
}

#[test]
fn decode_response_keeps_the_native_slot_only_when_native() {
    let bytes = br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served"}"#;
    let litellm: DecodedOcrResponse<TestPayload> =
        decode_response(bytes, false).expect("the body decodes");
    assert_eq!(litellm.native, None);
    assert_eq!(litellm.data.pages[0].index, 0);
    let native: DecodedOcrResponse<TestPayload> =
        decode_response(bytes, true).expect("the body decodes");
    assert!(native.native.is_some());
    assert!(native.text.contains("hello"));
}

#[test]
fn decode_request_value_names_the_failing_field() {
    let value = Json::Object(object(&[("pages", Json::Text("not a list".to_owned()))]));
    let error = decode_request_value::<TestPayload>(&value, "guardrail.document")
        .expect_err("a mistyped field fails");
    assert!(matches!(
        error,
        OcrError::RequestField { path } if path.starts_with("guardrail.document.")
    ));
}

#[test]
fn compose_body_merges_unknown_options_and_extra_body() {
    let params = object(&[
        ("pages", Json::List(Vec::new())),
        ("future_option", Json::Bool(true)),
        ("req_format", Json::Text("litellm".to_owned())),
        (
            "extra_body",
            Json::Object(object(&[(
                "provider_option",
                Json::Text("value".to_owned()),
            )])),
        ),
    ]);
    let body = TestBody {
        model: "requested-model".to_owned(),
        document: document(),
    };
    let composed = compose_body(&params, &body, &["pages"]).expect("the body composes");
    let Json::Object(fields) = composed else {
        panic!("the composed body is an object");
    };
    assert!(fields.contains_key("model"));
    assert!(fields.contains_key("document"));
    assert_eq!(fields.get("future_option"), Some(&Json::Bool(true)));
    assert_eq!(
        fields.get("provider_option"),
        Some(&Json::Text("value".to_owned()))
    );
    assert!(fields.get("pages").is_none());
    assert!(fields.get("req_format").is_none());
}
