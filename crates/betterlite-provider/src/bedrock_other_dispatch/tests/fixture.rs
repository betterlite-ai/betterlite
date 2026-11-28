use std::collections::{BTreeMap, VecDeque};
use std::error::Error as StdError;
use std::sync::{Arc, Mutex};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::count_call::counter::TokenCountResponse;
use betterlite_bedrock::count_call::handler::{BedrockCountTokensHandler, CountTokensHandler};
use betterlite_bedrock::embedding_call::EmbeddingCallResponse;
use betterlite_bedrock::embedding_call::ParamMap;
use betterlite_bedrock::transcription::binding::{TranscriptionBinding, TranscriptionResponse};
use betterlite_bedrock::transcription::call::NativeTranscription;
use betterlite_bedrock::transcription::error::CallError;
use betterlite_bedrock::transcription::params::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use indexmap::IndexMap;
use serde_json::Number;

use crate::bedrock_image_dispatch::BedrockImageResult;
use crate::bedrock_other_dispatch::BedrockOtherRequest;
use crate::bedrock_other_dispatch::BedrockOtherResult;
use crate::bedrock_other_dispatch::call::BedrockOtherOperation;
use crate::bedrock_other_dispatch::error::BedrockOtherError;
use crate::bedrock_other_dispatch::execute::{LogContext, Services, dispatch};
use crate::bedrock_other_dispatch::test_support::client::{FixtureClient, Replies, Reply};
use crate::bedrock_other_dispatch::test_support::factory::{ClientCall, Factory};
use crate::bedrock_other_dispatch::test_support::record::{Recorded, Seen};
use crate::bedrock_other_dispatch::test_support::source::Source;
use crate::bedrock_other_dispatch::test_support::{DiscardingSink, Prepared, prepost_env};
use crate::operation::{OperationRequest, OperationResult};

pub(crate) type Handler = BedrockCountTokensHandler<FixtureClient, Source>;
pub(crate) type Binding = NativeTranscription<FixtureClient, Source>;

// The scripted factory, transport and identity one dispatch runs against
pub(crate) struct Fixture {
    factory: Factory,
    transport: FixtureClient,
    source: Source,
}

impl Fixture {
    pub(crate) fn new(replies: &[Reply], credentials: Credentials) -> Self {
        let seen = Seen::default();
        let mut factory = Factory::queued(replies);
        factory.seen = seen.clone();
        Self {
            factory,
            transport: FixtureClient {
                replies: Replies(Arc::new(Mutex::new(
                    replies.iter().cloned().collect::<VecDeque<Reply>>(),
                ))),
                seen,
            },
            source: Source::new(credentials),
        }
    }

    pub(crate) fn source(&self) -> &Source {
        &self.source
    }

    pub(crate) fn client(&self) -> FixtureClient {
        FixtureClient {
            replies: self.factory.replies.clone(),
            seen: self.factory.seen.clone(),
        }
    }
}

// What one dispatch observed: its outcome, every posted request, and the logging state it wrote
pub(crate) struct Outcome<E>
where
    E: StdError + 'static,
{
    pub(crate) result: Result<BedrockOtherResult, BedrockOtherError<E>>,
    pub(crate) requests: Vec<Recorded>,
    pub(crate) calls: Vec<ClientCall>,
    pub(crate) regions: Vec<String>,
    pub(crate) state: LoggingState,
}

impl<E> Outcome<E>
where
    E: StdError + 'static,
{
    pub(crate) fn embedding(&self) -> &EmbeddingCallResponse {
        match &self.result {
            Ok(BedrockOtherResult::Embedding(response)) => response.as_ref(),
            Ok(_other) => panic!("the embedding arm returns an embedding reply"),
            Err(error) => panic!("the embedding dispatch failed: {error}"),
        }
    }

    pub(crate) fn status_body(&self) -> &Json {
        match &self.result {
            Ok(BedrockOtherResult::EmbeddingStatus(status)) => status.as_ref(),
            Ok(_other) => panic!("the status arm returns a status body"),
            Err(error) => panic!("the status dispatch failed: {error}"),
        }
    }

    pub(crate) fn rerank(&self) -> &RerankResponse {
        match &self.result {
            Ok(BedrockOtherResult::Rerank(response)) => response.as_ref(),
            Ok(_other) => panic!("the rerank arm returns a rerank reply"),
            Err(error) => panic!("the rerank dispatch failed: {error}"),
        }
    }

    pub(crate) fn count(&self) -> Option<&TokenCountResponse> {
        match &self.result {
            Ok(BedrockOtherResult::Count(count)) => count.as_deref(),
            Ok(_other) => panic!("the count arm returns a token count"),
            Err(error) => panic!("the count dispatch failed: {error}"),
        }
    }

    pub(crate) fn transcription(&self) -> &TranscriptionResponse {
        match &self.result {
            Ok(BedrockOtherResult::Transcription(response)) => response.as_ref(),
            Ok(_other) => panic!("the transcription arm returns a transcript"),
            Err(error) => panic!("the transcription dispatch failed: {error}"),
        }
    }

    pub(crate) fn image(&self) -> &BedrockImageResult {
        match &self.result {
            Ok(BedrockOtherResult::Image(result)) => result.as_ref(),
            Ok(_other) => panic!("the image arm returns an image result"),
            Err(error) => panic!("the image dispatch failed: {error}"),
        }
    }

    pub(crate) fn error(&self) -> &BedrockOtherError<E> {
        match &self.result {
            Ok(_other) => panic!("the dispatch should fail"),
            Err(error) => error,
        }
    }

    pub(crate) fn posted(&self) -> &Recorded {
        assert_eq!(self.requests.len(), 1, "one dispatch posts one request");
        &self.requests[0]
    }

    pub(crate) fn body(&self, index: usize) -> Json {
        serde_json::from_slice(&self.requests[index].body).expect("the request body is JSON")
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

    pub(crate) fn post_call(&self) -> bool {
        self.state
            .model_call_details
            .contains_key("original_response")
    }
}

pub(crate) fn value(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture value is JSON")
}

pub(crate) fn params(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("the optional params are a JSON object")
}

pub(crate) fn object(value: &Json) -> &IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the value is a JSON object");
        }
    }
}

pub(crate) fn array(value: &Json) -> &[Json] {
    match value {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("the value is a JSON array");
        }
    }
}

pub(crate) fn number(value: &Json) -> &Number {
    match value {
        Json::Number(number) => number,
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            panic!("the value is a JSON number");
        }
    }
}

pub(crate) fn image_params(source: &str) -> ImageParams {
    serde_json::from_str(source).expect("the image params are a JSON object")
}

pub(crate) fn transcription_params(source: &str) -> Params {
    serde_json::from_str(source).expect("the transcription params are a JSON object")
}

pub(crate) fn environment(source: &str) -> BTreeMap<String, String> {
    serde_json::from_str(source).expect("the environment is a string map")
}

pub(crate) async fn run(
    fixture: &Fixture,
    operation: BedrockOtherOperation<'_, '_, '_, Handler, Binding>,
) -> Outcome<CallError> {
    exchange(fixture, operation).await
}

pub(crate) async fn run_transcription<B>(
    fixture: &Fixture,
    operation: BedrockOtherOperation<'_, '_, '_, Handler, B>,
) -> Outcome<B::Error>
where
    B: TranscriptionBinding + Send + Sync,
{
    exchange(fixture, operation).await
}

async fn exchange<H, B>(
    fixture: &Fixture,
    operation: BedrockOtherOperation<'_, '_, '_, H, B>,
) -> Outcome<B::Error>
where
    H: CountTokensHandler,
    B: TranscriptionBinding + Send + Sync,
{
    let mut prepared = Prepared::new().expect("the fixture dependencies build");
    let sink = DiscardingSink;
    let env = prepost_env(&prepared.state, &prepared.redactor, &sink);
    let services = Services {
        secrets: &prepared.secrets,
        region_worker: &prepared.workers,
        source: &fixture.source,
        factory: &fixture.factory,
        transport: &fixture.transport,
        encoding: &prepared.encoding,
        signing: &prepared.signing,
        clock: Arc::clone(&prepared.clock),
        files: &prepared.files,
    };
    let mut log = LogContext {
        state: &mut prepared.state,
        env: &env,
    };
    // A dispatcher that answered another operation result would be a bug, and the fixture reports it as one
    let result = match dispatch(
        OperationRequest::BedrockOther(BedrockOtherRequest),
        operation,
        &services,
        &mut log,
    )
    .await
    {
        Ok(OperationResult::BedrockOther(result)) => Ok(result),
        Ok(_other) => panic!("the non-chat dispatcher returns a non-chat result"),
        Err(error) => Err(error),
    };
    Outcome {
        result,
        requests: fixture.factory.seen.requests(),
        calls: fixture.factory.calls(),
        regions: fixture.source.regions(),
        state: prepared.state,
    }
}
