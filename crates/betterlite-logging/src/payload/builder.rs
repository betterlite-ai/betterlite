use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;
use crate::logging_failure::{FailurePayloadInput, FailurePayloadSource};
use crate::logging_metadata::base64::Base64Truncator;
use crate::logging_state::LoggingState;
use crate::logging_success::ports::StandardPayloadBuilder;
use crate::payload::compose::{PayloadComposition, PayloadRequest, standard_logging_payload};
use crate::payload::deps::{PayloadSettings, PayloadSources};

pub const WEB_SEARCH_OPTIONS_KEY: &str = "web_search_options";
pub const FILE_SEARCH_KEY: &str = "file_search";

pub struct PayloadBuilderConfig {
    pub sources: Arc<PayloadSources>,
    pub settings: PayloadSettings,
    pub redactor: Arc<Redactor>,
    pub normalizer: Arc<Normalizer>,
    pub truncator: Base64Truncator,
    pub privacy: Settings,
}

pub struct PayloadBuilder {
    pub(crate) sources: Arc<PayloadSources>,
    pub(crate) settings: PayloadSettings,
    pub(crate) redactor: Arc<Redactor>,
    pub(crate) normalizer: Arc<Normalizer>,
    pub(crate) truncator: Base64Truncator,
    pub(crate) privacy: Settings,
}

impl PayloadBuilder {
    #[must_use]
    pub fn new(config: PayloadBuilderConfig) -> Self {
        Self {
            sources: config.sources,
            settings: config.settings,
            redactor: config.redactor,
            normalizer: config.normalizer,
            truncator: config.truncator,
            privacy: config.privacy,
        }
    }

    fn composition(&self) -> PayloadComposition<'_> {
        PayloadComposition {
            sources: &self.sources,
            settings: &self.settings,
            redactor: &self.redactor,
            normalizer: &self.normalizer,
            truncator: &self.truncator,
            privacy: self.privacy,
        }
    }

    // The builder never fails a request: a payload that cannot be built is warned about and skipped.
    fn assemble(&self, request: &PayloadRequest<'_>) -> Option<StandardLoggingPayload> {
        match standard_logging_payload(request, &self.composition()) {
            Ok(payload) => Some(payload),
            Err(error) => {
                tracing::warn!(%error, "the standard logging payload could not be built");
                None
            }
        }
    }
}

// initialize_standard_built_in_tools_params always carries both keys, with None for an absent value.
fn standard_built_in_tools(state: &LoggingState) -> Json {
    let entry = |key: &str| {
        state
            .standard_built_in_tools_params
            .get(key)
            .cloned()
            .unwrap_or(Json::Null)
    };
    Json::Object(IndexMap::from([
        (
            WEB_SEARCH_OPTIONS_KEY.to_owned(),
            entry(WEB_SEARCH_OPTIONS_KEY),
        ),
        (FILE_SEARCH_KEY.to_owned(), entry(FILE_SEARCH_KEY)),
    ]))
}

impl StandardPayloadBuilder for PayloadBuilder {
    fn build(
        &self,
        state: &LoggingState,
        response: &Json,
        start_time: DateTime<FixedOffset>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let tools = standard_built_in_tools(state);
        self.assemble(&PayloadRequest {
            state,
            init_response_obj: response,
            start_time,
            end_time,
            status: LoggingStatus::Success,
            error_str: None,
            original_exception: None,
            standard_built_in_tools_params: Some(&tools),
        })
    }
}

impl FailurePayloadSource for PayloadBuilder {
    fn build(&self, input: FailurePayloadInput<'_>) -> Option<StandardLoggingPayload> {
        let tools = standard_built_in_tools(input.state);
        // the failure helper passes init_response_obj={}, which has no body and no hidden params
        let empty = Json::Object(IndexMap::new());
        self.assemble(&PayloadRequest {
            state: input.state,
            init_response_obj: &empty,
            start_time: input.start_time,
            end_time: input.end_time,
            status: LoggingStatus::Failure,
            error_str: Some(input.error_str),
            original_exception: Some(input.exception),
            standard_built_in_tools_params: Some(&tools),
        })
    }
}
