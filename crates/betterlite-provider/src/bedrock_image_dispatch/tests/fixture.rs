use std::str;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::canvas_results::COST_HEADER;
use betterlite_bedrock::image_call::plan::{ImageEnvironment, LogContext};
use betterlite_bedrock::image_call::transport::{ClientKind, ImageReply, ImageRequest};
use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage};
use betterlite_bedrock::image_edits::response::EditReply;
use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;

use crate::bedrock_image_dispatch::call::{
    BedrockImageOperation, EditOperation, GenerateOperation, Transport,
};
use crate::bedrock_image_dispatch::error::BedrockImageError;
use crate::bedrock_image_dispatch::execute::dispatch;
use crate::bedrock_image_dispatch::test_support::{
    DiscardingSink, Factory, Reply, Seen, Source, instant, logging_state, prepost_env,
};
use crate::bedrock_image_dispatch::{BedrockImageRequest, BedrockImageResult};
use crate::operation::OperationRequest;

pub(crate) use crate::bedrock_image_dispatch::test_support::{Catalog, NOW_SECONDS};
pub(crate) use betterlite_bedrock::image_edits::plan::EditPlan;
pub(crate) use betterlite_bedrock::model_routes::ParamMap;

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

// The operation the caller resolved: a field left `None` or `false` is what the arm omits
pub(crate) struct GenerateFixture {
    pub(crate) model: Option<String>,
    pub(crate) prompt: String,
    pub(crate) optional_params: Params,
    pub(crate) extra_headers: Option<Vec<(String, String)>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) aimg_generation: bool,
}

impl GenerateFixture {
    pub(crate) fn new(model: Option<&str>, prompt: &str, optional_params: Params) -> Self {
        Self {
            model: model.map(str::to_owned),
            prompt: prompt.to_owned(),
            optional_params,
            extra_headers: None,
            timeout: None,
            aimg_generation: false,
        }
    }

    pub(crate) fn operation(&self) -> GenerateOperation<'_> {
        GenerateOperation {
            model: self.model.as_deref(),
            prompt: &self.prompt,
            optional_params: self.optional_params.clone(),
            extra_headers: self.extra_headers.as_deref(),
            api_base: None,
            api_key: None,
            timeout: self.timeout,
            aimg_generation: self.aimg_generation,
            environment: ImageEnvironment::default(),
            now: instant(NOW_SECONDS).expect("the fixture instant is after the Unix epoch"),
            created: NOW_SECONDS,
        }
    }
}

pub(crate) struct EditFixture {
    pub(crate) model: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) images: Vec<EditImage>,
    pub(crate) optional_params: ImageParams,
    pub(crate) extra_headers: Option<Vec<(String, String)>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) aimage_edit: bool,
    pub(crate) now: SystemTime,
}

impl EditFixture {
    pub(crate) fn new(
        model: Option<&str>,
        prompt: Option<&str>,
        images: Vec<EditImage>,
        optional_params: ImageParams,
    ) -> Self {
        Self {
            model: model.map(str::to_owned),
            prompt: prompt.map(str::to_owned),
            images,
            optional_params,
            extra_headers: None,
            timeout: None,
            aimage_edit: false,
            now: instant(NOW_SECONDS).expect("the fixture instant is after the Unix epoch"),
        }
    }

    pub(crate) fn operation<'fixture, 'cost, 'context>(
        &'fixture self,
        plan: EditPlan<'cost, 'context>,
    ) -> EditOperation<'fixture, 'cost, 'context> {
        EditOperation {
            model: self.model.as_deref(),
            prompt: self.prompt.as_deref(),
            images: &self.images,
            optional_params: self.optional_params.clone(),
            extra_headers: self.extra_headers.as_deref(),
            api_base: None,
            api_key: None,
            timeout: self.timeout,
            aimage_edit: self.aimage_edit,
            environment: EditEnvironment::default(),
            now: self.now,
            plan,
        }
    }
}

pub(crate) struct Run {
    result: Result<BedrockImageResult, BedrockImageError>,
    calls: Vec<(ClientKind, Option<Duration>)>,
    requests: Vec<ImageRequest>,
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

    pub(crate) fn posted(&self) -> &ImageRequest {
        assert_eq!(self.requests.len(), 1, "one dispatch posts one request");
        &self.requests[0]
    }

    pub(crate) fn text(&self) -> &str {
        str::from_utf8(&self.posted().body).expect("the request body is UTF-8")
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
    run_request(
        OperationRequest::BedrockImage(BedrockImageRequest),
        operation,
        reply,
    )
    .await
}

pub(crate) async fn run_request(
    request: OperationRequest,
    operation: BedrockImageOperation<'_, '_, '_>,
    reply: Reply,
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
    let source = Source {
        credentials: Credentials {
            access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
            session_token: None,
        },
    };
    let result = {
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
                supplied: None,
            },
            &mut log,
        )
        .await
    };
    Run {
        result,
        calls: factory.calls(),
        requests: factory.seen_requests(),
        state,
    }
}
