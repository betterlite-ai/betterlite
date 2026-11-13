#![cfg(test)]

use std::sync::Mutex;

use betterlite_interactions::types::interaction::InteractionInput;
use betterlite_interactions::types::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
    InteractionsStreamChunk,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::config::BaseInteractionsAPIConfig;
use super::error::{BaseLlmException, InteractionsContractError};
use super::records::{
    CompleteUrlRequest, CreateInteractionRequest, EnvironmentRequest, InteractionIdRequest,
    InteractionsLogging, InteractionsUpstream, Params, PreparedRequest, RequestHeaders,
    ResponseHeaders,
};

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] InteractionsContractError),
    #[error("the upstream body is not an interaction")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Default)]
struct Fixture {
    routes: Mutex<Vec<String>>,
}

impl Fixture {
    fn record(&self, route: String) {
        if let Ok(mut routes) = self.routes.lock() {
            routes.push(route);
        }
    }

    fn routes(&self) -> Vec<String> {
        self.routes
            .lock()
            .map_or_else(|_| Vec::new(), |routes| routes.clone())
    }

    fn decode<T>(
        &self,
        model: Option<&str>,
        label: &str,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<T, FixtureError>
    where
        T: DeserializeOwned,
    {
        self.record(format!(
            "{label}|{}|{}",
            raw_response.status_code,
            raw_response.headers.len()
        ));
        logging
            .model_call_details
            .insert(String::from("model"), text(model));
        serde_json::from_slice(&raw_response.body).map_err(|source| FixtureError::Decode { source })
    }

    fn deleted(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
        interaction_id: &str,
    ) -> Result<DeleteInteractionResult, FixtureError> {
        let mut result: DeleteInteractionResult =
            self.decode(None, "delete_response", raw_response, logging)?;
        result.id = Some(interaction_id.to_owned());
        logging
            .model_call_details
            .insert(String::from("deleted"), text(Some(interaction_id)));
        Ok(result)
    }

    fn chunk(
        &self,
        model: Option<&str>,
        parsed_chunk: &JsonValue,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsStreamChunk, FixtureError> {
        self.record(format!(
            "stream|{}|{}",
            option_text(model),
            matches!(parsed_chunk, JsonValue::Object(_))
        ));
        logging
            .model_call_details
            .insert(String::from("model"), text(model));
        serde_json::from_str(r#"{"id": "chunk_1"}"#)
            .map_err(|source| FixtureError::Decode { source })
    }
}

impl BaseInteractionsAPIConfig for Fixture {
    type Error = FixtureError;

    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn get_supported_params(&self, model: &str) -> Vec<String> {
        vec![model.to_owned(), String::from("tools")]
    }

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(environment_route(request));
        Ok(request.headers.clone())
    }

    fn get_complete_url(&self, request: &CompleteUrlRequest<'_>) -> Result<String, FixtureError> {
        self.record(url_route(request));
        Ok(format!("{}/interactions", option_text(request.api_base)))
    }

    fn transform_request(
        &self,
        request: &CreateInteractionRequest<'_>,
    ) -> Result<Params, FixtureError> {
        self.record(create_route(request));
        Ok(request.optional_params.clone())
    }

    fn transform_response(
        &self,
        model: Option<&str>,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, FixtureError> {
        self.decode(model, "response", raw_response, logging)
    }

    fn transform_streaming_response(
        &self,
        model: Option<&str>,
        parsed_chunk: &JsonValue,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsStreamChunk, FixtureError> {
        self.chunk(model, parsed_chunk, logging)
    }

    fn transform_get_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(lifecycle_request(self, "get", request, ""))
    }

    fn transform_get_response(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, FixtureError> {
        self.decode(None, "get_response", raw_response, logging)
    }

    fn transform_delete_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(lifecycle_request(self, "delete", request, ""))
    }

    fn transform_delete_response(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
        interaction_id: &str,
    ) -> Result<DeleteInteractionResult, FixtureError> {
        self.deleted(raw_response, logging, interaction_id)
    }

    fn transform_cancel_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(lifecycle_request(self, "cancel", request, ":cancel"))
    }

    fn transform_cancel_response(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<CancelInteractionResult, FixtureError> {
        self.decode(None, "cancel_response", raw_response, logging)
    }
}

fn option_text(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

fn text(value: Option<&str>) -> JsonValue {
    value.map_or(JsonValue::Null, |name| JsonValue::Text(name.to_owned()))
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("params should parse")
}

fn environment_route(request: &EnvironmentRequest<'_>) -> String {
    format!(
        "environment|{}|{}",
        request.model,
        request.litellm_params.is_some()
    )
}

fn url_route(request: &CompleteUrlRequest<'_>) -> String {
    let litellm = request.litellm_params.is_some();
    let stream = request.stream.unwrap_or(false);
    format!(
        "url|{}|{}|{}|{litellm}|{stream}",
        option_text(request.api_base),
        option_text(request.model),
        option_text(request.agent)
    )
}

fn create_route(request: &CreateInteractionRequest<'_>) -> String {
    let mut fields = vec![
        String::from("create"),
        option_text(request.model).to_owned(),
    ];
    fields.push(option_text(request.agent).to_owned());
    fields.push(request.input.is_some().to_string());
    fields.push(request.optional_params.len().to_string());
    fields.push(request.litellm_params.len().to_string());
    fields.push(request.headers.len().to_string());
    fields.join("|")
}

fn lifecycle_request(
    fixture: &Fixture,
    action: &str,
    request: &InteractionIdRequest<'_>,
    suffix: &str,
) -> PreparedRequest {
    fixture.record(format!(
        "{action}_request|{}|{}|{}|{}",
        request.interaction_id,
        request.api_base,
        request.litellm_params.len(),
        request.headers.len()
    ));
    PreparedRequest {
        url: format!("{}/{}{suffix}", request.api_base, request.interaction_id),
        params: Params::new(),
    }
}

fn request_headers() -> RequestHeaders {
    RequestHeaders::from([
        (
            String::from("x-goog-api-key"),
            SecretString::from(String::from("key")),
        ),
        (
            String::from("x-extra"),
            SecretString::from(String::from("yes")),
        ),
    ])
}

fn upstream(status_code: u16, body: &str) -> InteractionsUpstream {
    InteractionsUpstream {
        status_code,
        headers: ResponseHeaders::from([(String::from("x-request-id"), String::from("req_1"))]),
        body: body.as_bytes().to_vec(),
    }
}

fn id_request() -> (RequestHeaders, Params, String) {
    (
        request_headers(),
        params(r#"{"api_key": "key"}"#),
        String::from("https://example.test"),
    )
}

#[test]
fn base_capabilities() {
    let fixture = Fixture::default();
    assert_eq!(fixture.custom_llm_provider(), ProviderId::Gemini);
    assert_eq!(
        fixture.get_supported_params("gemini-2.5-flash"),
        vec![String::from("gemini-2.5-flash"), String::from("tools")]
    );
}

#[test]
fn environment_and_url_receive_every_parameter() {
    let fixture = Fixture::default();
    let headers = request_headers();
    let litellm_params = params(r#"{"api_key": "key"}"#);
    let environment = EnvironmentRequest {
        headers: &headers,
        model: "gemini-2.5-flash",
        litellm_params: Some(&litellm_params),
    };
    let validated = fixture
        .validate_environment(&environment)
        .expect("the fixture echoes its headers");
    assert_eq!(validated.len(), 2);
    assert_eq!(validated["x-goog-api-key"].expose_secret(), "key");

    let complete_url = CompleteUrlRequest {
        api_base: Some("https://example.test"),
        model: Some("m"),
        agent: Some("a"),
        litellm_params: Some(&litellm_params),
        stream: Some(true),
    };
    assert_eq!(
        fixture
            .get_complete_url(&complete_url)
            .expect("the fixture builds a collection url"),
        "https://example.test/interactions"
    );
    assert_eq!(
        fixture.routes(),
        vec![
            String::from("environment|gemini-2.5-flash|true"),
            String::from("url|https://example.test|m|a|true|true"),
        ]
    );
}

#[test]
fn create_transformation_receives_every_parameter() {
    let fixture = Fixture::default();
    let headers = request_headers();
    let optional_params = params(r#"{"store": false, "background": true}"#);
    let litellm_params = params(r#"{"api_key": "key"}"#);
    let input = InteractionInput::Text(String::from("hi"));
    let request = CreateInteractionRequest {
        model: Some("gemini-2.5-flash"),
        agent: Some("agent_1"),
        input: Some(&input),
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
    };
    let transformed = fixture
        .transform_request(&request)
        .expect("the fixture forwards its optional params");
    assert_eq!(transformed, optional_params);
    assert_eq!(
        fixture.routes(),
        vec![String::from("create|gemini-2.5-flash|agent_1|true|2|1|2")]
    );
}

#[test]
fn response_transformation_receives_the_upstream_and_logging() {
    let fixture = Fixture::default();
    let mut logging = InteractionsLogging::default();
    let raw_response = upstream(200, r#"{"id": "int_1", "status": "completed"}"#);
    let response = fixture
        .transform_response(Some("gemini-2.5-flash"), &raw_response, &mut logging)
        .expect("the upstream body decodes");
    assert_eq!(response.id.as_deref(), Some("int_1"));
    assert_eq!(
        logging.model_call_details.get("model"),
        Some(&JsonValue::Text(String::from("gemini-2.5-flash")))
    );
    assert_eq!(
        fixture.routes(),
        vec![String::from("response|gemini-2.5-flash|200|1")]
    );
}

#[test]
fn streaming_transformation_receives_the_parsed_chunk() {
    let fixture = Fixture::default();
    let mut logging = InteractionsLogging::default();
    let chunk = JsonValue::Object(IndexMap::from([(
        String::from("event_type"),
        JsonValue::Text(String::from("step.start")),
    )]));
    let streamed = fixture
        .transform_streaming_response(Some("m"), &chunk, &mut logging)
        .expect("the fixture answers a chunk");
    assert_eq!(streamed.id.as_deref(), Some("chunk_1"));
    assert!(logging.model_call_details.contains_key("model"));
    assert_eq!(fixture.routes(), vec![String::from("stream|m|true")]);
}

#[test]
fn get_requests_carry_the_interaction_id_and_response_context() {
    let fixture = Fixture::default();
    let (headers, litellm_params, api_base) = id_request();
    let request = InteractionIdRequest {
        interaction_id: "int_1",
        api_base: &api_base,
        litellm_params: &litellm_params,
        headers: &headers,
    };
    let prepared = fixture
        .transform_get_request(&request)
        .expect("the fixture builds a get url");
    assert_eq!(prepared.url, "https://example.test/int_1");
    let mut logging = InteractionsLogging::default();
    let response = fixture
        .transform_get_response(&upstream(200, r#"{"id": "int_1"}"#), &mut logging)
        .expect("the upstream body decodes");
    assert_eq!(response.id.as_deref(), Some("int_1"));
    assert_eq!(
        fixture.routes(),
        vec![
            String::from("get_request|int_1|https://example.test|1|2"),
            String::from("get_response|200|1"),
        ]
    );
}

#[test]
fn delete_requests_and_responses_carry_the_interaction_id() {
    let fixture = Fixture::default();
    let (headers, litellm_params, api_base) = id_request();
    let request = InteractionIdRequest {
        interaction_id: "int_1",
        api_base: &api_base,
        litellm_params: &litellm_params,
        headers: &headers,
    };
    let prepared = fixture
        .transform_delete_request(&request)
        .expect("the fixture builds a delete url");
    assert_eq!(prepared.url, "https://example.test/int_1");
    let mut logging = InteractionsLogging::default();
    let deleted = fixture
        .transform_delete_response(&upstream(200, "{}"), &mut logging, "int_1")
        .expect("the fixture answers the requested id");
    assert!(deleted.success);
    assert_eq!(deleted.id.as_deref(), Some("int_1"));
    assert_eq!(
        logging.model_call_details.get("deleted"),
        Some(&JsonValue::Text(String::from("int_1")))
    );
    assert_eq!(
        fixture.routes(),
        vec![
            String::from("delete_request|int_1|https://example.test|1|2"),
            String::from("delete_response|200|1"),
        ]
    );
}

#[test]
fn cancel_requests_and_responses_round_trip() {
    let fixture = Fixture::default();
    let (headers, litellm_params, api_base) = id_request();
    let request = InteractionIdRequest {
        interaction_id: "int_1",
        api_base: &api_base,
        litellm_params: &litellm_params,
        headers: &headers,
    };
    let prepared = fixture
        .transform_cancel_request(&request)
        .expect("the fixture builds a cancel url");
    assert_eq!(prepared.url, "https://example.test/int_1:cancel");
    let mut logging = InteractionsLogging::default();
    let cancelled = fixture
        .transform_cancel_response(
            &upstream(200, r#"{"id": "int_1", "status": "cancelled"}"#),
            &mut logging,
        )
        .expect("the upstream body decodes");
    assert_eq!(cancelled.status.as_deref(), Some("cancelled"));
    assert_eq!(
        fixture.routes(),
        vec![
            String::from("cancel_request|int_1|https://example.test|1|2"),
            String::from("cancel_response|200|1"),
        ]
    );
}

#[test]
fn default_error_class_keeps_the_status_message_and_headers() {
    let fixture = Fixture::default();
    let headers = ResponseHeaders::from([(String::from("retry-after"), String::from("3"))]);
    let error = fixture.get_error_class("upstream failed", 429, &headers);
    let FixtureError::Contract(InteractionsContractError::BaseLlm(BaseLlmException {
        status_code,
        message,
        headers: raised_headers,
    })) = error
    else {
        panic!("the raised error should be the base exception");
    };
    assert_eq!(status_code, 429);
    assert_eq!(message, "upstream failed");
    assert_eq!(raised_headers, headers);
}
