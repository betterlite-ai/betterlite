#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::assemble;
use crate::helicone::test_support::{event, json, logger};

fn provider_request(assembled: &super::Assembled) -> Json {
    let text = serde_json::to_string(&assembled.envelope).unwrap();
    let Json::Object(envelope) = json(&text) else {
        panic!("the envelope is not an object");
    };
    envelope["providerRequest"].clone()
}

fn request_meta(assembled: &super::Assembled) -> Json {
    let Json::Object(request) = provider_request(assembled) else {
        panic!("providerRequest is not an object");
    };
    request["meta"].clone()
}

#[test]
fn envelope_starts_the_metadata_with_helicone_auth() {
    let logger = logger("https://api.hconeai.com", Some("sk-key"));
    let key = SecretString::from("sk-key");
    let assembled = assemble(&logger.settings, &key, &event("gpt-4o")).unwrap();
    assert_eq!(
        request_meta(&assembled),
        json(r#"{"Helicone-Auth": "Bearer sk-key"}"#)
    );
}

#[test]
fn envelope_metadata_replaces_helicone_auth() {
    let logger = logger("https://api.hconeai.com", Some("sk-key"));
    let mut sent = event("gpt-4o");
    sent.litellm_params.metadata = IndexMap::from([(
        "Helicone-Auth".to_owned(),
        Json::Text("Bearer from-metadata".to_owned()),
    )]);
    let key = SecretString::from("sk-key");
    let assembled = assemble(&logger.settings, &key, &sent).unwrap();
    assert_eq!(
        request_meta(&assembled),
        json(r#"{"Helicone-Auth": "Bearer from-metadata"}"#)
    );
}

#[test]
fn envelope_sends_the_fallback_model_name() {
    let logger = logger("https://api.hconeai.com", Some("sk-key"));
    let key = SecretString::from("sk-key");
    let assembled = assemble(&logger.settings, &key, &event("mistral-large")).unwrap();
    let Json::Object(request) = provider_request(&assembled) else {
        panic!("providerRequest is not an object");
    };
    let Json::Object(body) = &request["json"] else {
        panic!("providerRequest.json is not an object");
    };
    assert_eq!(body["model"], json(r#""gpt-3.5-turbo""#));
}
