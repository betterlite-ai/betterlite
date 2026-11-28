#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
use betterlite_core::api::passthrough_call::aws::{AwsFlavor, AwsPassthrough as _};
use betterlite_core::api::passthrough_call::config::ConfigError;
use betterlite_models::resolution::ResolutionRequest;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::{BedrockSdkCall, NoCredentialChain};
use crate::aws_settings::sources::Credentials;
use crate::credentials::CredentialSource;
use crate::credentials::error::CredentialError;

const SIGNING_BASE: &str = "https://bedrock-runtime.eu-west-1.amazonaws.com/model/m/converse";
const MODEL_ARN: &str = "arn:aws:bedrock:us-east-2:123456789012:application-inference-profile/abc";

fn fixture_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(pairs: &[(&str, &str)]) -> Object {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), JsonValue::String((*value).to_owned())))
        .collect()
}

// One call the source answered: the key it was asked for and the region it signs in
#[derive(Debug, Clone)]
struct Resolved {
    access_key_id: Option<String>,
    region: String,
}

#[derive(Default)]
struct FakeSource {
    calls: Mutex<Vec<Resolved>>,
    access_key: String,
}

impl FakeSource {
    fn recorded(&self) -> Vec<Resolved> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CredentialSource for FakeSource {
    async fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> Result<Credentials, CredentialError> {
        let access_key_id = settings
            .access_key_id
            .as_ref()
            .map(|key| key.expose_secret().to_owned());
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Resolved {
                access_key_id,
                region: region.clone(),
            });
        Ok(Credentials {
            access_key_id: SecretString::from(self.access_key.clone()),
            secret_access_key: SecretString::from("fixture-secret"),
            session_token: None,
        })
    }
}

fn build(pairs: &[(&str, &str)]) -> BedrockSdkCall<FakeSource> {
    build_with(FakeSource::default(), pairs)
}

fn build_with(source: FakeSource, pairs: &[(&str, &str)]) -> BedrockSdkCall<FakeSource> {
    BedrockSdkCall::from_environment(&environment(pairs), source, fixture_time).unwrap()
}

fn exposed(token: Option<&SecretString>) -> Option<&str> {
    token.map(ExposeSecret::expose_secret)
}

fn url_request<'text>(
    model: &'text str,
    endpoint: &'text str,
    params: &'text Object,
    api_base: Option<&'text str>,
) -> PassthroughUrlRequest<'text> {
    PassthroughUrlRequest {
        api_base,
        api_key: None,
        target: PassthroughTarget {
            model,
            custom_llm_provider: "bedrock",
            endpoint,
        },
        request_query_params: None,
        litellm_params: params,
    }
}

fn url_of(
    config: &BedrockSdkCall<FakeSource>,
    flavor: AwsFlavor,
    request: &PassthroughUrlRequest<'_>,
) -> String {
    config.complete_url(flavor, request).unwrap().0.to_string()
}

fn signing<'text>(params: &'text Object) -> PassthroughSigning<'text> {
    PassthroughSigning {
        litellm_params: params,
        request_data: None,
        api_base: SIGNING_BASE,
        model: Some("anthropic.claude"),
    }
}

fn resolution<'text>(
    model: &'text str,
    api_base: Option<&'text str>,
    api_key: Option<&'text SecretString>,
) -> ResolutionRequest<'text> {
    ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base,
        api_key,
    }
}

#[test]
fn from_environment_reads_every_field() {
    let settings = build(&[
        ("AWS_REGION_NAME", "us-east-1"),
        ("AWS_REGION", "us-west-2"),
        ("AWS_DEFAULT_REGION", "eu-west-1"),
        ("AWS_BEDROCK_RUNTIME_ENDPOINT", "https://endpoint.example"),
        ("BEDROCK_MANTLE_API_BASE", "https://mantle.example/v1"),
        ("BEDROCK_MANTLE_API_KEY", "mantle-key"),
        ("BEDROCK_MANTLE_REGION", "eu-central-1"),
        ("AWS_BEARER_TOKEN_BEDROCK", "bearer-token"),
    ]);
    assert_eq!(
        settings.environment.region_name.as_deref(),
        Some("us-east-1")
    );
    assert_eq!(settings.environment.region.as_deref(), Some("us-west-2"));
    assert_eq!(
        settings.environment.session_region.as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        settings.environment.runtime_endpoint.as_deref(),
        Some("https://endpoint.example")
    );
    assert_eq!(
        settings.globals.mantle_api_base.as_deref(),
        Some("https://mantle.example/v1")
    );
    assert_eq!(
        settings.globals.mantle_region.as_deref(),
        Some("eu-central-1")
    );
    assert_eq!(
        settings.globals.aws_region_name.as_deref(),
        Some("us-east-1")
    );
    assert_eq!(settings.globals.aws_region.as_deref(), Some("us-west-2"));
    assert_eq!(
        exposed(settings.globals.mantle_api_key.as_ref()),
        Some("mantle-key")
    );
    assert_eq!(
        exposed(settings.globals.aws_bearer_token.as_ref()),
        Some("bearer-token")
    );
}

#[test]
fn bedrock_url_region_chain_follows_the_environment() {
    let config = build(&[
        ("AWS_REGION_NAME", "us-east-1"),
        ("AWS_REGION", "eu-central-1"),
        ("AWS_DEFAULT_REGION", "eu-west-1"),
    ]);
    let empty = Object::new();
    let explicit = object(&[("aws_region_name", "ap-south-1")]);
    let request = url_request("anthropic.claude", "model/x/converse", &explicit, None);
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.ap-south-1.amazonaws.com/model/x/converse"
    );
    let request = url_request("anthropic.claude", "model/x/converse", &empty, None);
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/x/converse"
    );
    let region_only = build(&[("AWS_REGION", "eu-central-1")]);
    assert_eq!(
        url_of(&region_only, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.eu-central-1.amazonaws.com/model/x/converse"
    );
    let session_only = build(&[("AWS_DEFAULT_REGION", "eu-west-1")]);
    assert_eq!(
        url_of(&session_only, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/x/converse"
    );
    let default = build(&[]);
    assert_eq!(
        url_of(&default, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/x/converse"
    );
    let invalid = build(&[("AWS_REGION_NAME", "Not_A_Region")]);
    let error = invalid
        .complete_url(AwsFlavor::Bedrock, &request)
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid AWS region format: Not_A_Region");
}

#[test]
fn bedrock_url_endpoint_precedence() {
    let config = build(&[(
        "AWS_BEDROCK_RUNTIME_ENDPOINT",
        "https://environment.example",
    )]);
    let empty = Object::new();
    let request = url_request("anthropic.claude", "model/x/converse", &empty, None);
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &request),
        "https://environment.example/model/x/converse"
    );
    let addressed = url_request(
        "anthropic.claude",
        "model/x/converse",
        &empty,
        Some("https://base.example"),
    );
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &addressed),
        "https://base.example/model/x/converse"
    );
    let configured = object(&[("aws_bedrock_runtime_endpoint", "https://configured.example")]);
    let request = url_request("anthropic.claude", "model/x/converse", &configured, None);
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &request),
        "https://configured.example/model/x/converse"
    );
    let addressed = url_request(
        "anthropic.claude",
        "model/x/converse",
        &configured,
        Some("https://base.example"),
    );
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &addressed),
        "https://base.example/model/x/converse"
    );
    let default = build(&[]);
    let request = url_request("anthropic.claude", "model/x/converse", &empty, None);
    assert_eq!(
        url_of(&default, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/x/converse"
    );
}

// the model_id param is ARN-encoded into the path, and an explicit aws_region_name picks the host
#[test]
fn bedrock_url_rewrites_the_model_id_and_takes_the_region_param() {
    let config = build(&[]);
    let params = object(&[("aws_region_name", "eu-central-1"), ("model_id", MODEL_ARN)]);
    let request = url_request("anthropic.claude", "model/foo/converse", &params, None);
    let probe = format!("/model/{MODEL_ARN}/converse");
    let encoded = encode_runtime_modelid_arn(&probe);
    let segment = encoded.split('/').nth(2).unwrap_or("");
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &request),
        format!("https://bedrock-runtime.eu-central-1.amazonaws.com/model/{segment}/converse")
    );
}

// LlmRegion: a model ARN names the region even when no aws_region_name is passed
#[test]
fn bedrock_url_takes_the_region_from_the_model_arn() {
    let config = build(&[]);
    let empty = Object::new();
    let request = url_request(MODEL_ARN, "model/foo/converse", &empty, None);
    assert_eq!(
        url_of(&config, AwsFlavor::Bedrock, &request),
        "https://bedrock-runtime.us-east-2.amazonaws.com/model/foo/converse"
    );
}

#[test]
fn mantle_url_takes_the_region_from_the_base_host_or_the_environment() {
    let config = build(&[("BEDROCK_MANTLE_REGION", "eu-central-1")]);
    let host = "https://bedrock-mantle.eu-west-1.api.aws/v1";
    let params = object(&[("api_base", host)]);
    let request = url_request(
        "openai.gpt-oss-120b",
        "chat/completions",
        &params,
        Some(host),
    );
    assert_eq!(
        url_of(&config, AwsFlavor::Mantle, &request),
        "https://bedrock-runtime.eu-west-1.amazonaws.com/chat/completions"
    );
    let empty = Object::new();
    let request = url_request("openai.gpt-oss-120b", "chat/completions", &empty, None);
    assert_eq!(
        url_of(&config, AwsFlavor::Mantle, &request),
        "https://bedrock-runtime.eu-central-1.amazonaws.com/chat/completions"
    );
    let plain = "http://127.0.0.1:9";
    let params = object(&[("api_base", plain)]);
    let request = url_request(
        "openai.gpt-oss-120b",
        "chat/completions",
        &params,
        Some(plain),
    );
    assert_eq!(
        url_of(&config, AwsFlavor::Mantle, &request),
        "http://127.0.0.1:9/chat/completions"
    );
}

#[test]
fn bedrock_bearer_token_signs_with_the_environment_token() {
    let config = build(&[("AWS_BEARER_TOKEN_BEDROCK", "env-token")]);
    let empty = Object::new();
    let (headers, body) = config
        .sign_request(AwsFlavor::Bedrock, Headers::new(), &signing(&empty))
        .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer env-token")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(body.as_deref(), Some(br"{}".as_slice()));
    assert!(config.source.recorded().is_empty());
}

#[test]
fn an_empty_bearer_token_falls_through_to_sigv4() {
    let source = FakeSource {
        access_key: String::from("AKIAEMPTY"),
        ..FakeSource::default()
    };
    let config = build_with(source, &[("AWS_BEARER_TOKEN_BEDROCK", "")]);
    let empty = Object::new();
    let (headers, _body) = config
        .sign_request(AwsFlavor::Bedrock, Headers::new(), &signing(&empty))
        .unwrap();
    let authorization = headers.get("Authorization").unwrap();
    assert!(
        authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAEMPTY/20231114/us-west-2/bedrock/aws4_request"
        ),
        "{authorization}"
    );
}

#[test]
fn mantle_bearer_chain_prefers_the_params_then_the_environment() {
    let config = build(&[
        ("BEDROCK_MANTLE_API_KEY", "mantle-key"),
        ("AWS_BEARER_TOKEN_BEDROCK", "aws-token"),
    ]);
    let empty = Object::new();
    let params = object(&[("api_key", "param-key")]);
    let (headers, body) = config
        .sign_request(AwsFlavor::Mantle, Headers::new(), &signing(&params))
        .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer param-key")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(body.as_deref(), Some(br"{}".as_slice()));
    let (headers, _body) = config
        .sign_request(AwsFlavor::Mantle, Headers::new(), &signing(&empty))
        .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer mantle-key")
    );
    let fallback = build(&[
        ("BEDROCK_MANTLE_API_KEY", ""),
        ("AWS_BEARER_TOKEN_BEDROCK", "aws-token"),
    ]);
    let (headers, _body) = fallback
        .sign_request(AwsFlavor::Mantle, Headers::new(), &signing(&empty))
        .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer aws-token")
    );
}

#[test]
fn sigv4_signs_with_the_source_at_the_signing_region() {
    let source = FakeSource {
        access_key: String::from("AKIAFIXTURE"),
        ..FakeSource::default()
    };
    let config = build_with(source, &[("AWS_REGION_NAME", "us-east-1")]);
    let params = object(&[
        ("aws_region_name", "eu-west-1"),
        ("aws_access_key_id", "AKIAPARAM"),
        ("aws_secret_access_key", "fixture-secret"),
    ]);
    let (headers, _body) = config
        .sign_request(AwsFlavor::Bedrock, Headers::new(), &signing(&params))
        .unwrap();
    let authorization = headers.get("Authorization").unwrap();
    assert!(
        authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAFIXTURE/20231114/eu-west-1/bedrock/aws4_request"
        ),
        "{authorization}"
    );
    let calls = config.source.recorded();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].region, "eu-west-1");
    assert_eq!(calls[0].access_key_id.as_deref(), Some("AKIAPARAM"));
}

#[test]
fn mantle_sigv4_signs_at_the_mantle_region() {
    let source = FakeSource {
        access_key: String::from("AKIAMANTLE"),
        ..FakeSource::default()
    };
    let config = build_with(source, &[("BEDROCK_MANTLE_REGION", "eu-central-1")]);
    let params = object(&[("aws_secret_access_key", "fixture-secret")]);
    let (headers, _body) = config
        .sign_request(AwsFlavor::Mantle, Headers::new(), &signing(&params))
        .unwrap();
    let authorization = headers.get("Authorization").unwrap();
    assert!(
        authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAMANTLE/20231114/eu-central-1/bedrock/aws4_request"
        ),
        "{authorization}"
    );
    let calls = config.source.recorded();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].region, "eu-central-1");
}

#[test]
fn no_credential_chain_is_a_value_error() {
    let config =
        BedrockSdkCall::from_environment(&environment(&[]), NoCredentialChain, fixture_time)
            .unwrap();
    let empty = Object::new();
    let error = config
        .sign_request(AwsFlavor::Bedrock, Headers::new(), &signing(&empty))
        .unwrap_err();
    assert!(matches!(&error, ConfigError::ValueError { .. }));
    assert_eq!(error.to_string(), "AWS credential provider is unavailable");
}

#[test]
fn mantle_endpoint_derives_the_base_from_the_model_region_prefix() {
    let config = build(&[]);
    let request = resolution("bedrock_mantle/us-east-2/openai.gpt-oss-120b", None, None);
    let endpoint = config.mantle_endpoint(&request).unwrap();
    assert_eq!(
        endpoint.api_base,
        "https://bedrock-mantle.us-east-2.api.aws/v1"
    );
    assert_eq!(endpoint.base_model, "openai.gpt-oss-120b");
    assert!(endpoint.api_key.is_none());
    // split_mantle_region_prefix: a head that names no Bedrock region stays part of the model id
    let trailing = resolution("bedrock_mantle/openai.gpt-oss-120b/extra", None, None);
    let endpoint = config.mantle_endpoint(&trailing).unwrap();
    assert_eq!(endpoint.base_model, "openai.gpt-oss-120b/extra");
    assert_eq!(
        endpoint.api_base,
        "https://bedrock-mantle.us-east-1.api.aws/v1"
    );
}

#[test]
fn mantle_endpoint_keeps_the_configured_base_and_the_key_chain() {
    let config = build(&[
        ("BEDROCK_MANTLE_API_BASE", "https://mantle.example/v1"),
        ("BEDROCK_MANTLE_API_KEY", "mantle-key"),
        ("AWS_BEARER_TOKEN_BEDROCK", "aws-token"),
    ]);
    let key = SecretString::from("request-key");
    let request = resolution(
        "bedrock_mantle/us-east-1/openai.gpt-oss-120b",
        None,
        Some(&key),
    );
    let endpoint = config.mantle_endpoint(&request).unwrap();
    assert_eq!(endpoint.api_base, "https://mantle.example/v1");
    assert_eq!(exposed(endpoint.api_key.as_ref()), Some("request-key"));
    assert_eq!(endpoint.base_model, "openai.gpt-oss-120b");
    let request = resolution("bedrock_mantle/openai.gpt-oss-120b", None, None);
    assert_eq!(
        exposed(config.mantle_endpoint(&request).unwrap().api_key.as_ref()),
        Some("mantle-key")
    );
    let fallback = build(&[
        ("BEDROCK_MANTLE_API_BASE", ""),
        ("BEDROCK_MANTLE_API_KEY", ""),
        ("BEDROCK_MANTLE_REGION", "eu-central-1"),
        ("AWS_BEARER_TOKEN_BEDROCK", "aws-token"),
    ]);
    let endpoint = fallback.mantle_endpoint(&request).unwrap();
    assert_eq!(
        endpoint.api_base,
        "https://bedrock-mantle.eu-central-1.api.aws/v1"
    );
    assert_eq!(exposed(endpoint.api_key.as_ref()), Some("aws-token"));
}

#[test]
fn mantle_endpoint_region_chain_follows_the_environment() {
    let request = resolution("bedrock_mantle/openai.gpt-oss-120b", None, None);
    let name_only = build(&[("AWS_REGION_NAME", "us-east-2")]);
    assert_eq!(
        name_only.mantle_endpoint(&request).unwrap().api_base,
        "https://bedrock-mantle.us-east-2.api.aws/v1"
    );
    let region_only = build(&[("AWS_REGION", "us-west-2")]);
    assert_eq!(
        region_only.mantle_endpoint(&request).unwrap().api_base,
        "https://bedrock-mantle.us-west-2.api.aws/v1"
    );
    let default = build(&[]);
    assert_eq!(
        default.mantle_endpoint(&request).unwrap().api_base,
        "https://bedrock-mantle.us-east-1.api.aws/v1"
    );
}

#[test]
fn mantle_endpoint_refuses_an_invalid_environment_region() {
    let config = build(&[("BEDROCK_MANTLE_REGION", "Not_A_Region")]);
    let request = resolution("bedrock_mantle/openai.gpt-oss-120b", None, None);
    let error = config.mantle_endpoint(&request).unwrap_err();
    assert!(matches!(&error, ConfigError::ValueError { .. }));
    assert!(error.to_string().contains("Not_A_Region"), "{error}");
}
