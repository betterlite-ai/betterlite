#![cfg(test)]

use super::{edit, edit_headers, edit_timeout, outbound};
use crate::completion::setup::outbound_client;
use crate::images::convert::parse;
use crate::images::session::{CallSetup, ImageSession, call_setup};
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::credentials::OpenAiCredentials;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_setup::RequestMap;
use rstest::rstest;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
async fn edit_sends_multipart_request_and_decodes_response() {
    let serving = serve(
        reply("200 OK", &[], r#"{"created":7,"data":[]}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut blobs = vec![vec![1_u8, 2_u8]];
    let payload = format!(
        r#"{{"arguments":{{"prompt":"cat","api_key":"sk-123456789012345678901234567890","api_base":"http://{}/v1","image":{{"$file":{{"name":"cat.png","blob":0}}}}}}}}"#,
        serving.address
    );
    let (call, files) = parse(&payload, &mut blobs).unwrap();
    let environment = BTreeMap::new();
    let session = ImageSession {
        client: outbound_client(&environment).unwrap(),
        environment,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let answer = edit(&session, call, files).await.unwrap();
    assert_eq!(answer.reply.created, 7);
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/images/edits HTTP/1.1");
    assert!(sent.head.contains("content-type: multipart/form-data"));
    assert!(
        sent.head
            .contains("authorization: Bearer sk-123456789012345678901234567890")
    );
    assert!(sent.body.contains("filename=\"cat.png\""));
    assert!(sent.body.contains("\r\n\r\n\u{1}\u{2}\r\n"));
}

#[rstest]
#[case::default(None, None, 6000)]
#[case::environment(Some("17"), None, 17)]
#[case::empty_environment(Some(""), None, 6000)]
#[case::explicit(Some("17"), Some(3), 3)]
#[case::zero(Some("17"), Some(0), 0)]
fn edit_deadline(#[case] env: Option<&str>, #[case] given: Option<u64>, #[case] seconds: u64) {
    let mut blobs = vec![vec![1_u8]];
    let payload = format!(
        r#"{{"arguments":{{"prompt":"cat","api_key":"key","timeout":{},"image":{{"$file":{{"name":"cat.png","blob":0}}}}}}}}"#,
        serde_json::to_string(&given).unwrap()
    );
    let (call, files) = parse(&payload, &mut blobs).unwrap();
    let mut environment = BTreeMap::new();
    if let Some(value) = env {
        environment.insert(String::from("REQUEST_TIMEOUT"), value.to_owned());
    }
    let session = ImageSession {
        client: outbound_client(&environment).unwrap(),
        environment,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let setup = call_setup(
        &session,
        &call.arguments,
        &call.settings,
        "dall-e-2",
        "openai",
    )
    .unwrap();
    let request = outbound(&session, &call, &files, "dall-e-2", &setup).unwrap();
    assert_eq!(request.deadline, Duration::from_secs(seconds));
}

#[rstest]
#[case::canonical("Authorization")]
#[case::lowercase("authorization")]
#[case::mixed("aUtHoRiZaTiOn")]
fn edit_headers_keep_caller_auth(#[case] name: &str) {
    let setup = CallSetup {
        headers: BTreeMap::from([(name.to_owned(), String::from("Bearer caller"))]),
        timeout: Duration::from_secs(600),
        credentials: OpenAiCredentials {
            api_base: String::from("https://example.test"),
            api_key: Some(SecretString::from("configured-key")),
            organization: None,
        },
    };
    let headers = edit_headers(&setup).unwrap();
    assert_eq!(headers.get("authorization").unwrap(), "Bearer caller");
    assert!(headers.get("authorization").unwrap().is_sensitive());
}

#[rstest]
#[case::invalid("soon")]
#[case::negative("-1")]
fn edit_timeout_rejects_bad_env(#[case] value: &str) {
    let environment = BTreeMap::from([(String::from("REQUEST_TIMEOUT"), value.to_owned())]);
    let error = edit_timeout(&RequestMap::new(), &environment).unwrap_err();
    assert!(!error.to_string().is_empty());
}
