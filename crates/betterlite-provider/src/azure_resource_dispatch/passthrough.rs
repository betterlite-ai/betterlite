use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::error::AzurePassthroughError;
use betterlite_azure::passthrough::logging::{
    AzureRelayedLog, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayedResponse,
};

// The route-logging seams B3636 owns stay at their default collector
pub type Config = AzurePassthroughConfig<NoChatReplyProjection, NoChatStreamAssembly>;

pub struct Dispatch<'call> {
    pub config: &'call Config,
    pub logging: &'call mut dyn PassthroughLogging,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    StreamingRequest {
        endpoint: &'call str,
        request_data: &'call Object,
    },
    CompleteUrl(PassthroughUrlRequest<'call>),
    NonStreamingResponse {
        target: PassthroughTarget<'call>,
        response: RelayedResponse<'call>,
        request_data: &'call Object,
    },
    CollectedChunks {
        chunks: &'call [String],
        target: PassthroughTarget<'call>,
    },
}

pub enum Outcome {
    Streaming(bool),
    Url { url: String, base: String },
    Logged(Option<Box<AzureRelayedLog>>),
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzurePassthroughError>> + Send + 'call>>;

impl Dispatch<'_> {
    // B548's preparation and its two logging projections, each one calling the leaf collector
    pub fn run(self) -> Result<Outcome, AzurePassthroughError> {
        let Self {
            config,
            logging,
            call,
        } = self;
        Ok(match call {
            Call::StreamingRequest {
                endpoint,
                request_data,
            } => Outcome::Streaming(config.is_streaming_request(endpoint, request_data)),
            Call::CompleteUrl(request) => {
                let (url, base) = config.get_complete_url(&request)?;
                Outcome::Url {
                    url: url.to_string(),
                    base,
                }
            }
            Call::NonStreamingResponse {
                target,
                response,
                request_data,
            } => Outcome::Logged(
                config
                    .logging_non_streaming_response(&target, &response, request_data, logging)
                    .map(Box::new),
            ),
            Call::CollectedChunks { chunks, target } => Outcome::Logged(
                config
                    .handle_collected_chunks(chunks, logging, &target)
                    .map(Box::new),
            ),
        })
    }
}

pub fn dispatch(dispatch: Dispatch<'_>) -> RunFuture<'_> {
    Box::pin(ready(dispatch.run()))
}
