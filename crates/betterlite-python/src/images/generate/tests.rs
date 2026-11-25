#![cfg(test)]

use super::generate;
use crate::completion::setup::outbound_client;
use crate::images::convert::parse;
use crate::images::session::ImageSession;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
async fn generate_sends_prompt_and_decodes_response() {
    let serving = serve(
        reply("200 OK", &[], r#"{"created":11,"data":[]}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut blobs = Vec::new();
    let payload = format!(
        r#"{{"arguments":{{"prompt":"a cat","model":"dall-e-2","api_key":"sk-123456789012345678901234567890","api_base":"http://{}/v1"}}}}"#,
        serving.address
    );
    let (call, _files) = parse(&payload, &mut blobs).unwrap();
    let environment = BTreeMap::new();
    let session = ImageSession {
        client: outbound_client(&environment).unwrap(),
        environment,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let answer = generate(&session, call).await.unwrap();
    assert_eq!(answer.reply.created, 11);
    assert_eq!(
        answer.hidden_params.get("model"),
        Some(&Json::Text(String::from("dall-e-2")))
    );
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/images/generations HTTP/1.1");
    assert!(sent.body.contains("\"prompt\":\"a cat\""));
}
