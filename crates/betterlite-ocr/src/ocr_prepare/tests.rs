#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::json::{Json, Object};
use crate::ocr_config::connection::OcrInputSource;
use crate::ocr_config::{OCR_INLINE_MAX_BYTES, Secrets};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::tests::support::ocr_client;
use crate::ocr_input::{LiteLLMOcrRequest, OcrConnectionInputs, OcrDocumentInput};
use crate::ocr_prepare::{
    encode_file_document, mime_type_for_name, prepare_document, prepare_request, read_path_document,
};
use crate::ocr_provider::OcrConfigKind;
use crate::ocr_settings::OcrSettings;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!("betterlite-ocr-prepare-{}-{unique}", process::id()));
    fs::create_dir_all(&dir).expect("the test directory is created");
    dir
}

fn inline_document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn secrets(entries: &[(&str, &str)]) -> Secrets {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

fn resolved_request(
    model: &str,
    custom_llm_provider: Option<&str>,
    document: OcrDocument,
    connection: OcrConnectionInputs,
    options: Object,
) -> LiteLLMOcrRequest<OcrDocument> {
    LiteLLMOcrRequest::from_inputs(
        model,
        OcrDocumentInput::Document(document),
        custom_llm_provider,
        options,
        connection,
    )
    .expect("the request builds")
    .map_document(prepare_document)
    .expect("the document resolves")
}

#[rstest]
#[case::pdf("document.pdf", "application/pdf")]
#[case::png("image.png", "image/png")]
#[case::jpg("photo.jpg", "image/jpeg")]
#[case::jpeg("photo.jpeg", "image/jpeg")]
#[case::gif("animation.gif", "image/gif")]
#[case::webp("image.webp", "image/webp")]
#[case::tiff("scan.tiff", "image/tiff")]
#[case::tif("scan.tif", "image/tiff")]
#[case::bmp("bitmap.bmp", "image/bmp")]
#[case::uppercase("DOCUMENT.PDF", "application/pdf")]
#[case::guessed_text("notes.txt", "text/plain")]
#[case::unknown("file.unknown-extension", "application/octet-stream")]
fn file_names_map_to_their_mime_type(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(mime_type_for_name(name), expected);
}

#[test]
fn file_bytes_are_encoded_by_mime_type() {
    assert_eq!(
        encode_file_document(b"abc", Some("scan.png"), None).expect("the image encodes"),
        OcrDocument::ImageUrl {
            image_url: "data:image/png;base64,YWJj".to_owned(),
            extra: BTreeMap::new(),
        }
    );
    assert_eq!(
        encode_file_document(b"abc", Some("scan.png"), Some("application/pdf"))
            .expect("the PDF encodes"),
        inline_document("data:application/pdf;base64,YWJj")
    );
    assert_eq!(
        encode_file_document(b"abc", None, None).expect("the bytes encode"),
        inline_document("data:application/octet-stream;base64,YWJj")
    );
}

#[test]
fn file_encoding_rejects_empty_and_oversized_bytes() {
    assert!(matches!(
        encode_file_document(b"", Some("scan.png"), None),
        Err(OcrError::EmptyFile)
    ));
    let limit = vec![b'a'; OCR_INLINE_MAX_BYTES];
    let document =
        encode_file_document(&limit, Some("scan.pdf"), None).expect("the cap is allowed");
    assert!(
        document
            .source()
            .starts_with("data:application/pdf;base64,")
    );
    let mut oversized = limit;
    oversized.push(b'a');
    assert!(matches!(
        encode_file_document(&oversized, Some("scan.pdf"), None),
        Err(OcrError::InlineDocumentTooLarge)
    ));
}

#[rstest]
#[case::parameter("text/plain;bad")]
#[case::extra_slash("text/plain/extra")]
#[case::leading_space(" text/plain")]
#[case::newline("text/plain\n")]
#[case::empty("")]
#[case::no_slash("text")]
#[case::no_kind("/plain")]
#[case::no_subtype("text/")]
fn invalid_mime_types_are_rejected(#[case] mime: &str) {
    assert!(matches!(
        encode_file_document(b"abc", None, Some(mime)),
        Err(OcrError::InvalidMimeType(_))
    ));
}

#[test]
fn path_documents_are_read_with_their_extension() {
    let dir = temp_dir();
    let path = dir.join("scan.png");
    fs::write(&path, b"abc").expect("the file is written");
    assert_eq!(
        read_path_document(&path, None).expect("the path reads"),
        OcrDocument::ImageUrl {
            image_url: "data:image/png;base64,YWJj".to_owned(),
            extra: BTreeMap::new(),
        }
    );
    assert_eq!(
        read_path_document(&path, Some("application/pdf")).expect("the path reads"),
        inline_document("data:application/pdf;base64,YWJj")
    );
    let empty = dir.join("empty.png");
    fs::write(&empty, b"").expect("the empty file is written");
    assert!(matches!(
        read_path_document(&empty, None),
        Err(OcrError::EmptyFile)
    ));
    fs::remove_dir_all(&dir).expect("the test directory is removed");

    let missing = dir.join("missing.pdf");
    let error = read_path_document(&missing, None).expect_err("the missing path fails");
    let OcrError::FileRead {
        path: read_path,
        source,
    } = error
    else {
        panic!("a missing path is a file read error");
    };
    assert_eq!(read_path, missing);
    assert_eq!(source.kind(), ErrorKind::NotFound);
}

#[test]
fn prepare_document_passes_documents_through_and_reads_paths() {
    let document = inline_document("data:application/pdf;base64,YWJj");
    assert_eq!(
        prepare_document(OcrDocumentInput::Document(document)).expect("the document passes"),
        inline_document("data:application/pdf;base64,YWJj")
    );
    let dir = temp_dir();
    let path = dir.join("scan.pdf");
    fs::write(&path, b"abc").expect("the file is written");
    assert_eq!(
        prepare_document(OcrDocumentInput::Path {
            path,
            mime_type: None,
        })
        .expect("the path reads"),
        inline_document("data:application/pdf;base64,YWJj")
    );
    fs::remove_dir_all(&dir).expect("the test directory is removed");
}

#[test]
fn prepare_document_encodes_bytes_with_their_name() {
    assert_eq!(
        prepare_document(OcrDocumentInput::Bytes {
            bytes: b"abc".to_vec(),
            file_name: Some("scan.png".to_owned()),
            mime_type: None,
        })
        .expect("the bytes encode"),
        OcrDocument::ImageUrl {
            image_url: "data:image/png;base64,YWJj".to_owned(),
            extra: BTreeMap::new(),
        }
    );
    assert_eq!(
        prepare_document(OcrDocumentInput::Bytes {
            bytes: b"abc".to_vec(),
            file_name: Some("scan.png".to_owned()),
            mime_type: Some("application/pdf".to_owned()),
        })
        .expect("the explicit type wins"),
        inline_document("data:application/pdf;base64,YWJj")
    );
    assert!(matches!(
        prepare_document(OcrDocumentInput::Bytes {
            bytes: Vec::new(),
            file_name: None,
            mime_type: None,
        }),
        Err(OcrError::EmptyFile)
    ));
}

#[test]
fn prepare_request_resolves_the_provider_and_the_environment() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = resolved_request(
        "azure_ai/pixtral-12b",
        None,
        inline_document("data:application/pdf;base64,YWJj"),
        OcrConnectionInputs::default(),
        Object::new(),
    );
    let prepared = prepare_request(
        request,
        true,
        &client,
        secrets(&[
            ("AZURE_AI_API_BASE", "https://azure.example.com"),
            ("AZURE_AI_API_KEY", "azure-key"),
        ]),
    );
    assert_eq!(prepared.model, "pixtral-12b");
    assert!(prepared.caller_document);
    let api_base = prepared.connection.api_base.expect("the base resolves");
    assert_eq!(api_base, "https://azure.example.com");
    assert_eq!(
        prepared.connection.api_base_source,
        OcrInputSource::Environment
    );
    let api_key = prepared.connection.api_key.expect("the key resolves");
    assert_eq!(api_key.expose_secret(), "azure-key");
    assert_eq!(
        prepared.connection.api_key_source,
        OcrInputSource::Environment
    );
}

#[test]
fn prepare_request_prefers_the_mistral_azure_names() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = resolved_request(
        "mistral-ocr-latest",
        None,
        inline_document("data:application/pdf;base64,YWJj"),
        OcrConnectionInputs::default(),
        Object::new(),
    );
    let prepared = prepare_request(
        request,
        false,
        &client,
        secrets(&[
            ("MISTRAL_AZURE_API_KEY", "azure-key"),
            ("MISTRAL_AZURE_API_BASE", "https://azure.example.com"),
            ("MISTRAL_API_KEY", "plain-key"),
        ]),
    );
    let api_key = prepared.connection.api_key.expect("the key resolves");
    assert_eq!(api_key.expose_secret(), "azure-key");
    assert_eq!(
        prepared.connection.api_key_source,
        OcrInputSource::Environment
    );
    let api_base = prepared.connection.api_base.expect("the base resolves");
    assert_eq!(api_base, "https://azure.example.com");
    assert_eq!(
        prepared.connection.api_base_source,
        OcrInputSource::Environment
    );
}

#[test]
fn prepare_request_falls_back_to_the_provider_key_and_ignores_empty_values() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = resolved_request(
        "mistral-ocr-latest",
        None,
        inline_document("data:application/pdf;base64,YWJj"),
        OcrConnectionInputs::default(),
        Object::new(),
    );
    let prepared = prepare_request(
        request,
        false,
        &client,
        secrets(&[
            ("MISTRAL_AZURE_API_KEY", ""),
            ("MISTRAL_API_KEY", "plain-key"),
        ]),
    );
    let api_key = prepared.connection.api_key.expect("the key resolves");
    assert_eq!(api_key.expose_secret(), "plain-key");
}

#[test]
fn prepare_request_keeps_request_credentials_and_their_source() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let connection = OcrConnectionInputs {
        api_key: Some(SecretString::from("request-key")),
        api_base: Some("https://request.example.com".to_owned()),
        extra_headers: Object::new(),
        timeout: None,
        input_sources: BTreeMap::from([
            ("api_key".to_owned(), OcrInputSource::Request),
            ("api_base".to_owned(), OcrInputSource::Request),
        ]),
    };
    let request = resolved_request(
        "mistral-ocr-latest",
        None,
        inline_document("data:application/pdf;base64,YWJj"),
        connection,
        Object::new(),
    );
    let prepared = prepare_request(
        request,
        false,
        &client,
        secrets(&[
            ("MISTRAL_AZURE_API_KEY", "azure-key"),
            ("MISTRAL_AZURE_API_BASE", "https://azure.example.com"),
        ]),
    );
    let api_key = prepared.connection.api_key.expect("the key is kept");
    assert_eq!(api_key.expose_secret(), "request-key");
    assert_eq!(prepared.connection.api_key_source, OcrInputSource::Request);
    assert_eq!(
        prepared.connection.api_base.as_deref(),
        Some("https://request.example.com")
    );
    assert_eq!(prepared.connection.api_base_source, OcrInputSource::Request);
}

#[rstest]
#[case::cohere("parse", Some("cohere"), OcrConfigKind::Cohere)]
#[case::textract(
    "detect-document-text",
    Some("aws_textract"),
    OcrConfigKind::AwsTextract
)]
#[case::reducto("parse-v3", Some("reducto"), OcrConfigKind::ReductoV3)]
#[case::doc_intel(
    "documentintelligence/prebuilt-read",
    Some("azure_ai"),
    OcrConfigKind::AzureDocumentIntelligence
)]
fn prepare_request_resolves_each_kind_through_its_custom_provider(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: OcrConfigKind,
) {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = resolved_request(
        model,
        provider,
        inline_document("data:application/pdf;base64,YWJj"),
        OcrConnectionInputs::default(),
        Object::new(),
    );
    assert_eq!(request.config, expected);
    let prepared = prepare_request(request, false, &client, Secrets::new());
    assert_eq!(prepared.model, model);
}

#[test]
fn prepare_request_keeps_the_options_and_the_connection_timeout() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let connection = OcrConnectionInputs {
        api_key: None,
        api_base: None,
        extra_headers: Object::new(),
        timeout: Some(Duration::from_secs(7)),
        input_sources: BTreeMap::new(),
    };
    let options = Object::from([("pages".to_owned(), Json::List(Vec::new()))]);
    let request = resolved_request(
        "mistral-ocr-latest",
        None,
        inline_document("data:application/pdf;base64,YWJj"),
        connection,
        options,
    );
    let prepared = prepare_request(request, false, &client, Secrets::new());
    assert_eq!(prepared.connection.timeout, Duration::from_secs(7));
    assert_eq!(prepared.optional_params.len(), 1);
    assert!(prepared.optional_params.contains_key("pages"));
}
