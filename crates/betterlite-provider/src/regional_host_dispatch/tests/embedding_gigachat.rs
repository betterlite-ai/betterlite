use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::embedding::call_for;
use super::support::{EmbeddingFixture, headers, secret};

// the GigaChat embedding chain: get_llm_provider puts a GIGACHAT_API_KEY env into the passed key, so the request key, that env, litellm.api_key, litellm.gigachat_key and the credentials env follow in that order
#[rstest]
#[case::request_key(
    [Some("sk-request"), Some("sk-env"), Some("sk-global"), Some("sk-section"), Some("sk-cred")],
    Some("sk-request")
)]
#[case::api_key_env_beats_the_globals(
    [None, Some("sk-env"), Some("sk-global"), Some("sk-section"), Some("sk-cred")],
    Some("sk-env")
)]
#[case::global_key(
    [None, None, Some("sk-global"), Some("sk-section"), Some("sk-cred")],
    Some("sk-global")
)]
#[case::section_key(
    [None, None, None, Some("sk-section"), Some("sk-cred")],
    Some("sk-section")
)]
#[case::credentials_env([None, None, None, None, Some("sk-cred")], Some("sk-cred"))]
#[case::no_key([None, None, None, None, None], None)]
fn gigachat_embedding_key_chain(
    #[case] sources: [Option<&str>; 5],
    #[case] expected: Option<&str>,
) {
    let [
        request_key,
        api_key_env,
        global_key,
        section_key,
        credentials_env,
    ] = sources;
    let mut fixture = EmbeddingFixture::baseline();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.gigachat_key = section_key.map(secret);
    fixture.settings.environment.gigachat_api_key = api_key_env.map(secret);
    fixture.settings.environment.gigachat_credentials = credentials_env.map(secret);
    let call = call_for(ProviderId::Gigachat, &fixture).expect("the arm answers");
    assert_eq!(
        call.api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        expected.map(str::to_owned)
    );
}

#[test]
fn gigachat_embedding_drops_the_headers_and_carries_only_ssl_verify() {
    let mut fixture = EmbeddingFixture::baseline();
    fixture.extra_headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.api_base = Some(String::from("https://gigachat.example/v1"));
    let call = call_for(ProviderId::Gigachat, &fixture).expect("the arm answers");
    assert_eq!(call.headers, None);
    assert_eq!(call.api_base, fixture.api_base);
    assert_eq!(call.litellm_params.len(), 1_usize);
    assert_eq!(call.litellm_params.get("ssl_verify"), Some(&Json::Null));
}

#[rstest]
#[case::absent(None, Json::Null)]
#[case::enabled(Some(true), Json::Bool(true))]
#[case::disabled(Some(false), Json::Bool(false))]
fn gigachat_embedding_carries_the_request_ssl_verify(
    #[case] ssl_verify: Option<bool>,
    #[case] expected: Json,
) {
    let mut fixture = EmbeddingFixture::baseline();
    fixture.ssl_verify = ssl_verify;
    let call = call_for(ProviderId::Gigachat, &fixture).expect("the arm answers");
    assert_eq!(call.litellm_params.get("ssl_verify"), Some(&expected));
}
