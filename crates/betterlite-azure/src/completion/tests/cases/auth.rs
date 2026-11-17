use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::completion::request::validate_environment;
use crate::completion::tests::fixture::ScriptedSource;
use crate::endpoints::projection::ClientKind;

use super::support::{async_buffered, buffered, sync_buffered, text};

fn environment_headers() {
    let key = SecretString::from("sk-unit");
    let token = SecretString::from("ad-token");
    let both = validate_environment(Some(&key), Some(&token));
    assert_eq!(both.0.len(), 2);
    let content = both.0.get("content-type").map(String::as_str);
    assert_eq!(content, Some("application/json"));
    assert_eq!(both.0.get("api-key").map(String::as_str), Some("sk-unit"));
    let token_only = validate_environment(None, Some(&token));
    let bearer = token_only.0.get("Authorization").map(String::as_str);
    assert_eq!(bearer, Some("Bearer ad-token"));
    assert!(!token_only.0.contains_key("api-key"));
    let empty = SecretString::from("");
    let empty_key = validate_environment(Some(&empty), Some(&token));
    assert_eq!(empty_key.0.get("api-key").map(String::as_str), Some(""));
    assert_eq!(validate_environment(None, None).0.len(), 1);
}

pub(super) fn azure_text_auth() {
    environment_headers();
    let mut token_call = sync_buffered("{}");
    token_call.parts.key = None;
    token_call.parts.token = Some(SecretString::from("ad-token"));
    let token_run = token_call.execute();
    let key_header = ["additional_args", "headers", "api_key"];
    assert_eq!(token_run.pre(&key_header), Json::Null);
    let token_header = ["additional_args", "headers", "azure_ad_token"];
    assert_eq!(text(&token_run.pre(&token_header)), "ad-token");
    let authorization = ["additional_args", "headers", "Authorization"];
    let mut keyless = async_buffered("{}");
    keyless.source.api_key = None;
    let keyless_run = keyless.execute();
    assert_eq!(text(&keyless_run.pre(&authorization)), "Bearer None");
    let mut supplied = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered());
    supplied.api_key = Some(SecretString::from("sk-supplied"));
    let mut caller = async_buffered("{}");
    caller.parts.supplied = Some(supplied.handle());
    let supplied_run = caller.execute();
    assert_eq!(
        text(&supplied_run.pre(&authorization)),
        "Bearer sk-supplied"
    );
    assert!(supplied_run.source.gets().is_empty());
}
