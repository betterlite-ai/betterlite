#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::provider_defaults_a::policies::MantleEndpoint;
use betterlite_models::resolution::ResolutionRequest;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughSigning, PassthroughUrlRequest,
};
use secrecy::SecretString;
use url::Url;

use super::super::PassthroughCallError;
use super::super::allm_passthrough_route;
use super::super::aws::{AwsFlavor, AwsPassthrough};
use super::super::config::ConfigError;
use super::super::logging;
use super::MODEL_ARN;
use super::support::{
    bedrock_args, empty_environment, logging_state, mantle_args, proxy_client, vllm_args,
};

// One call the fake port answered, with the input the test cares about
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    MantleEndpoint {
        model: String,
    },
    CompleteUrl {
        flavor: AwsFlavor,
        api_base: Option<String>,
    },
    Sign {
        flavor: AwsFlavor,
        api_key: Option<String>,
    },
}

struct FakeAws {
    base: String,
    mantle_error: Option<String>,
    sign_error: Option<String>,
    calls: Mutex<Vec<Call>>,
}

fn port(fake: &Arc<FakeAws>) -> Arc<dyn AwsPassthrough> {
    Arc::clone(fake)
}

fn fake(base: &str) -> Arc<FakeAws> {
    Arc::new(FakeAws {
        base: base.to_owned(),
        mantle_error: None,
        sign_error: None,
        calls: Mutex::new(Vec::new()),
    })
}

impl FakeAws {
    fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

#[async_trait]
impl AwsPassthrough for FakeAws {
    fn mantle_endpoint(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<MantleEndpoint, ConfigError> {
        self.record(Call::MantleEndpoint {
            model: request.model.to_owned(),
        });
        match &self.mantle_error {
            Some(message) => Err(ConfigError::ValueError {
                message: message.clone(),
            }),
            None => Ok(MantleEndpoint {
                api_base: self.base.clone(),
                api_key: Some(SecretString::from("port-key")),
                base_model: String::from("original"),
            }),
        }
    }

    fn complete_url(
        &self,
        flavor: AwsFlavor,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), ConfigError> {
        self.record(Call::CompleteUrl {
            flavor,
            api_base: request.api_base.map(str::to_owned),
        });
        let url =
            Url::parse(&format!("{}/{}", self.base, request.target.endpoint)).map_err(|error| {
                ConfigError::ValueError {
                    message: error.to_string(),
                }
            })?;
        Ok((url, self.base.clone()))
    }

    async fn sign_request(
        &self,
        flavor: AwsFlavor,
        _headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), ConfigError> {
        let api_key = context
            .litellm_params
            .get("api_key")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        self.record(Call::Sign { flavor, api_key });
        if let Some(message) = &self.sign_error {
            return Err(ConfigError::ValueError {
                message: message.clone(),
            });
        }
        let mut signed = Headers::new();
        signed.insert(
            String::from("Authorization"),
            String::from("Bearer port-signed"),
        );
        signed.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
        let body = context.request_data.map_or_else(
            || br"{}".to_vec(),
            |data| dumps_object(&logging::json_object(data)).into_bytes(),
        );
        Ok((signed, Some(body)))
    }
}

#[tokio::test]
async fn call_bedrock_sends_the_port_url_headers_and_body() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let fake = fake(&base);
    let mut args = bedrock_args(&base);
    args.json = Some(JsonValue::Object(super::json_object(&[(
        "model",
        "bedrock/anthropic.claude",
    )])));
    let answer = allm_passthrough_route(
        args,
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    assert_eq!(
        fake.calls(),
        vec![
            Call::CompleteUrl {
                flavor: AwsFlavor::Bedrock,
                api_base: Some(base),
            },
            Call::Sign {
                flavor: AwsFlavor::Bedrock,
                api_key: None,
            },
        ]
    );
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer port-signed"));
    assert!(sent.head.contains("content-type: application/json"));
    assert_eq!(sent.body, r#"{"model": "bedrock/anthropic.claude"}"#);
}

#[tokio::test]
async fn call_mantle_resolves_with_the_port_endpoint_and_logs_the_base_model() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let fake = fake(&base);
    let mut state = logging_state();
    let answer = allm_passthrough_route(
        mantle_args(&base),
        &mut state,
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    assert_eq!(state.model, "original");
    assert_eq!(
        fake.calls(),
        vec![
            Call::MantleEndpoint {
                model: String::from("bedrock_mantle/original"),
            },
            Call::CompleteUrl {
                flavor: AwsFlavor::Mantle,
                api_base: Some(base),
            },
            Call::Sign {
                flavor: AwsFlavor::Mantle,
                api_key: Some(String::from("port-key")),
            },
        ]
    );
    assert_eq!(
        serving.finish().unwrap().line,
        "POST /chat/completions HTTP/1.1"
    );
}

#[tokio::test]
async fn call_bedrock_application_inference_profile_is_arn_encoded() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let fake = fake(&base);
    let mut args = bedrock_args(&base);
    args.endpoint = format!("model/{MODEL_ARN}/converse");
    allm_passthrough_route(
        args,
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert!(
        sent.line.contains(":application-inference-profile%2Fabc"),
        "{}",
        sent.line
    );
}

#[tokio::test]
async fn call_vllm_never_asks_the_port() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}/v1", serving.address);
    let fake = fake(&base);
    allm_passthrough_route(
        vllm_args(&base),
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .expect("the upstream answers");
    assert!(fake.calls().is_empty());
}

// only the missing Mantle endpoint asks the port; another get_llm_provider failure keeps its resolution error
#[tokio::test]
async fn error_another_resolution_failure_never_asks_the_port() {
    let fake = fake("http://127.0.0.1:1");
    let mut args = vllm_args("http://127.0.0.1:1");
    args.model = String::from("azure_ai/gpt-4o");
    args.custom_llm_provider = None;
    args.api_base = Some(String::from("https://[abc]/v1"));
    let error = allm_passthrough_route(
        args,
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .err()
    .expect("the api_base is not a URL");
    assert!(matches!(error, PassthroughCallError::Resolution(_)));
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn error_a_failing_mantle_endpoint_is_reported_as_its_own_variant() {
    let fake = Arc::new(FakeAws {
        base: String::from("http://127.0.0.1:1"),
        mantle_error: Some(String::from("invalid AWS region format: Not_A_Region")),
        sign_error: None,
        calls: Mutex::new(Vec::new()),
    });
    let error = allm_passthrough_route(
        mantle_args("http://127.0.0.1:1"),
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .err()
    .expect("the endpoint is refused");
    assert!(matches!(error, PassthroughCallError::MantleEndpoint(_)));
    assert_eq!(error.to_string(), "invalid AWS region format: Not_A_Region");
    assert_eq!(
        fake.calls(),
        vec![Call::MantleEndpoint {
            model: String::from("bedrock_mantle/original"),
        }]
    );
}

#[tokio::test]
async fn error_a_failing_sign_is_a_mapped_synthesized_500() {
    let fake = Arc::new(FakeAws {
        base: String::from("http://127.0.0.1:1"),
        mantle_error: None,
        sign_error: Some(String::from("AWS credential provider is unavailable")),
        calls: Mutex::new(Vec::new()),
    });
    let error = allm_passthrough_route(
        bedrock_args("http://127.0.0.1:1"),
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .err()
    .expect("signing fails");
    let PassthroughCallError::Mapped(provider) = &error else {
        panic!("expected the provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "AWS credential provider is unavailable");
}

// B8690's kept bug: the port signs the caller's body, and the swapped model reaches the wire only when no port did
#[tokio::test]
async fn call_mantle_sends_the_body_the_port_signed_before_the_model_swap() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let fake = fake(&base);
    let mut args = mantle_args(&base);
    args.content = Some(bytes::Bytes::from_static(b"ignored"));
    args.json = Some(JsonValue::Object(super::json_object(&[(
        "model",
        "bedrock_mantle/original",
    )])));
    allm_passthrough_route(
        args,
        &mut logging_state(),
        &proxy_client(1024),
        &empty_environment(),
        &port(&fake),
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert_eq!(sent.body, r#"{"model": "bedrock_mantle/original"}"#);
    assert!(sent.head.contains("authorization: Bearer port-signed"));
}
