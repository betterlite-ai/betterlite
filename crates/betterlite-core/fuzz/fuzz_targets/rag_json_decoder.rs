#![no_main]

use std::collections::BTreeMap;

use betterlite_core::api::request_body::{ParsedJson, parse_json_text};
use betterlite_core::rag::ingest_request::{IngestBody, IngestRequest, parse_rag_ingest_request};
use betterlite_core::rag::upload_security::EicarTestMalwareScanner;
use libfuzzer_sys::fuzz_target;

const MAX_UPLOAD_BYTES: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = str::from_utf8(data) else {
        return;
    };
    let ParsedJson::Object(body) = parse_json_text(text) else {
        return;
    };
    let headers = BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Json(&body),
    };
    if let Ok(parsed) = parse_rag_ingest_request(
        &request,
        MAX_UPLOAD_BYTES,
        &EicarTestMalwareScanner,
        uuid::Uuid::nil,
    ) {
        assert!(parsed.ingest_options.contains_key("vector_store"));
        assert!(
            parsed
                .file
                .is_none_or(|file| file.content.len() <= MAX_UPLOAD_BYTES)
        );
    }
});
