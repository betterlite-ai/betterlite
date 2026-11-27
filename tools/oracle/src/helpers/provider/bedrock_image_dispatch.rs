use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::image_call::transport::{ImageReply, ImageRequest};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::bedrock_image_dispatch::call::{BedrockImageOperation, Transport};
use betterlite_provider::bedrock_image_dispatch::error::BedrockImageError;
use betterlite_provider::bedrock_image_dispatch::execute::dispatch;
use betterlite_provider::bedrock_image_dispatch::test_support::{
    DiscardingSink, Factory, Reply, Seen, Source, logging_state, prepost_env,
};
use betterlite_provider::bedrock_image_dispatch::{BedrockImageRequest, BedrockImageResult};
use betterlite_provider::operation::{OperationRequest, OperationResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use tokio::runtime::Builder;

use crate::openai_media_dispatch::{OTHER_OUTCOME, failure};

mod rows;
mod tests;

pub(crate) struct Op;

// The five operation families the composition recognises; each case names the one its probes run
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Case {
    TitanImage,
    StabilityImage,
    CanvasImage,
    CanvasEdit,
    StabilityEdit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) case: Case,
    #[serde(default)]
    pub(super) model_cost: ParamMap,
    pub(super) probes: Vec<Probe>,
}

// One row's call: the field a family never reads stays at its default
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Probe {
    pub(super) model: String,
    #[serde(default)]
    pub(super) prompt: Option<String>,
    #[serde(default)]
    pub(super) images: Vec<String>,
    #[serde(default)]
    pub(super) optional_params: ImageParams,
    #[serde(default)]
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) reply: ProbeReply,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ProbeReply {
    pub(super) status: u16,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

impl Default for ProbeReply {
    fn default() -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: String::from("{\"images\": [\"eA==\"]}"),
        }
    }
}

impl ProbeReply {
    pub(super) fn answer(&self) -> Reply {
        Reply::Answer(ImageReply {
            status: self.status,
            headers: self
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            body: self.body.as_bytes().to_vec(),
        })
    }
}

// What one dispatch observed: its outcome, every posted request, and the logging state it wrote
pub(super) struct Outcome {
    pub(super) result: Result<BedrockImageResult, BedrockImageError>,
    pub(super) requests: Vec<ImageRequest>,
    pub(super) state: LoggingState,
}

// The fixture transport and credential source every row runs the production dispatcher against
pub(super) async fn exchange(
    operation: BedrockImageOperation<'_, '_, '_>,
    reply: Reply,
    credentials: Credentials,
) -> Result<Outcome, Box<HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure("Redactor", error))?;
    let mut state = logging_state(&redactor).map_err(|error| failure("Logging", error))?;
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let factory = fixture(reply);
    let source = Source { credentials };
    let dispatched = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        dispatch_image(operation, &source, &factory, &mut log).await
    };
    Ok(Outcome {
        result: outcome(dispatched)?,
        requests: factory
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        state,
    })
}

fn fixture(reply: Reply) -> Factory {
    Factory {
        reply,
        calls: Mutex::new(Vec::new()),
        seen: Seen::default(),
    }
}

fn outcome(
    dispatched: Result<OperationResult, BedrockImageError>,
) -> Result<Result<BedrockImageResult, BedrockImageError>, Box<HelperError>> {
    match dispatched {
        Ok(OperationResult::BedrockImage(result)) => Ok(Ok(result)),
        Ok(_other) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Ok(Err(error)),
    }
}

async fn dispatch_image(
    operation: BedrockImageOperation<'_, '_, '_>,
    source: &Source,
    factory: &Factory,
    log: &mut LogContext<'_, '_>,
) -> Result<OperationResult, BedrockImageError> {
    dispatch(
        OperationRequest::BedrockImage(BedrockImageRequest),
        operation,
        source,
        Transport {
            factory,
            supplied: None,
        },
        log,
    )
    .await
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(rows::case_rows(input))
    }
}
