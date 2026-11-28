use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::canvas_results::COST_HEADER;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::image_call::transport::{ImageReply, ImageRequest};
use betterlite_bedrock::image_edits::response::EditReply;
use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use crate::bedrock_image_dispatch::call::{BedrockImageOperation, Transport};
use crate::bedrock_image_dispatch::error::BedrockImageError;
use crate::bedrock_image_dispatch::execute::dispatch;
use crate::bedrock_image_dispatch::test_support::{
    DiscardingSink, Factory, FixtureClient, Reply, Seen, Source, logging_state, prepost_env,
};
use crate::bedrock_image_dispatch::{BedrockImageRequest, BedrockImageResult};
use crate::operation::{OperationRequest, OperationResult};

// The prices and the Nova Canvas flag the edit selector reads; generation never consults it
const MODEL_COST: &str = r#"{
    "amazon.nova-canvas-v1:0": {
        "litellm_provider": "bedrock",
        "supports_nova_canvas_image_edit": true,
        "output_cost_per_image": 0.25
    },
    "stability.stable-image-inpaint-v1:0": {
        "litellm_provider": "bedrock",
        "output_cost_per_image": 0.07
    }
}"#;

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

pub(crate) fn model_cost() -> ParamMap {
    serde_json::from_str(MODEL_COST).expect("the model cost map is a JSON object")
}

pub(crate) fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("the optional params are a JSON object")
}

pub(crate) fn image_params(source: &str) -> ImageParams {
    serde_json::from_str(source).expect("the optional params are a JSON object")
}

pub(crate) fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer(ImageReply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    })
}

pub(crate) struct Run {
    result: Result<BedrockImageResult, BedrockImageError>,
    calls: Vec<(ClientKind, Option<Duration>)>,
    requests: Vec<ImageRequest>,
    supplied: Vec<ImageRequest>,
    state: LoggingState,
}

impl Run {
    pub(crate) fn generated(&self) -> ModelResponse {
        match self.result.as_ref() {
            Ok(BedrockImageResult::Generate(response)) => (**response).clone(),
            Ok(BedrockImageResult::Edit(_)) => panic!("the generation arm returns no edit reply"),
            Err(error) => panic!("the generation dispatch failed: {error}"),
        }
    }

    pub(crate) fn edited(&self) -> &EditReply {
        match self.result.as_ref() {
            Ok(BedrockImageResult::Edit(reply)) => reply,
            Ok(BedrockImageResult::Generate(_)) => {
                panic!("the edit arm returns no generated reply")
            }
            Err(error) => panic!("the edit dispatch failed: {error}"),
        }
    }

    pub(crate) fn error(&self) -> &BedrockImageError {
        match &self.result {
            Ok(_) => panic!("the dispatch should fail"),
            Err(error) => error,
        }
    }

    pub(crate) fn calls(&self) -> &[(ClientKind, Option<Duration>)] {
        &self.calls
    }

    pub(crate) fn requests(&self) -> &[ImageRequest] {
        &self.requests
    }

    pub(crate) fn supplied(&self) -> &[ImageRequest] {
        &self.supplied
    }

    pub(crate) fn posted(&self) -> &ImageRequest {
        assert_eq!(self.requests.len(), 1, "one dispatch posts one request");
        &self.requests[0]
    }

    pub(crate) fn generation_body(&self) -> Params {
        serde_json::from_slice(&self.posted().body).expect("the request body is a JSON object")
    }

    pub(crate) fn edit_body(&self) -> ImageParams {
        serde_json::from_slice(&self.posted().body).expect("the request body is a JSON object")
    }

    pub(crate) fn images(&self) -> Vec<String> {
        self.generated()
            .data
            .iter()
            .map(|image| image.b64_json.clone())
            .collect()
    }

    pub(crate) fn edit_images(&self) -> Vec<String> {
        self.edited()
            .response
            .data
            .iter()
            .flatten()
            .filter_map(|image| image.b64_json.clone())
            .collect()
    }

    pub(crate) fn created(&self) -> Option<i64> {
        self.generated().created
    }

    pub(crate) fn cost(&self) -> Option<f64> {
        match self.edited().additional_headers.get(COST_HEADER) {
            Some(Json::Number(number)) => number.as_f64(),
            Some(_other) => panic!("the response cost header is a number"),
            None => None,
        }
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.posted()
            .headers
            .iter()
            .find(|(key, _value)| key.eq_ignore_ascii_case(name))
            .map(|(_key, value)| value.as_str())
    }

    pub(crate) fn authorization(&self) -> Option<&str> {
        self.header("authorization")
    }

    pub(crate) fn post_call(&self) -> Option<&Json> {
        self.state.model_call_details.get("original_response")
    }

    pub(crate) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }
}

pub(crate) async fn run(operation: BedrockImageOperation<'_, '_, '_>, reply: Reply) -> Run {
    let request = OperationRequest::BedrockImage(BedrockImageRequest);
    run_with(request, operation, reply, None).await
}

pub(crate) async fn run_request(
    request: OperationRequest,
    operation: BedrockImageOperation<'_, '_, '_>,
    reply: Reply,
) -> Run {
    run_with(request, operation, reply, None).await
}

pub(crate) async fn run_supplied(
    operation: BedrockImageOperation<'_, '_, '_>,
    reply: Reply,
    kind: ClientKind,
) -> Run {
    let client = FixtureClient {
        reply: reply.clone(),
        seen: Seen::default(),
    };
    let supplied = SuppliedClient {
        kind,
        transport: &client,
    };
    run_with(
        OperationRequest::BedrockImage(BedrockImageRequest),
        operation,
        reply,
        Some(supplied),
    )
    .await
}

// The supplied client posts through its own recorder, so an empty factory log proves it was never asked
async fn run_with(
    request: OperationRequest,
    operation: BedrockImageOperation<'_, '_, '_>,
    reply: Reply,
    supplied: Option<SuppliedClient<'_, FixtureClient>>,
) -> Run {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    let mut state = logging_state(&redactor).expect("the fixture logging state builds");
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let factory = Factory {
        reply,
        calls: Mutex::new(Vec::new()),
        seen: Seen::default(),
    };
    let supplied_seen = supplied
        .as_ref()
        .map(|client| Arc::clone(&client.transport.seen));
    let source = fixture_source();
    let dispatched = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        dispatch(
            request,
            operation,
            &source,
            Transport {
                factory: &factory,
                supplied,
            },
            &mut log,
        )
        .await
    };
    finish(dispatched, &factory, supplied_seen, state)
}

// The dispatch outcome becomes the image record the caller reads, next to the requests each client saw
fn finish(
    dispatched: Result<OperationResult, BedrockImageError>,
    factory: &Factory,
    supplied_seen: Option<Seen>,
    state: LoggingState,
) -> Run {
    let result = match dispatched {
        Ok(OperationResult::BedrockImage(result)) => Ok(result),
        Ok(_other) => panic!("the image dispatcher returns an image result"),
        Err(error) => Err(error),
    };
    let supplied = supplied_seen.map_or_else(Vec::new, |seen| {
        seen.lock().unwrap_or_else(PoisonError::into_inner).clone()
    });
    Run {
        result,
        calls: factory
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        requests: factory
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        supplied,
        state,
    }
}

// The identity the fixture source answers with; the operation's own aws keys never reach the body
fn fixture_source() -> Source {
    Source {
        credentials: Credentials {
            access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
            session_token: None,
        },
    }
}
