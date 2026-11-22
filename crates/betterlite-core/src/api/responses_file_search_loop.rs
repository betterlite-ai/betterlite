mod decoder;
pub mod error;
pub mod follow_up;
pub mod ports;
pub mod records;
pub mod search;

mod tests;

use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};
use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_file_search_tools::outputs::{
    build_file_search_call_output, build_message_output, response_output_text,
};
use crate::api::responses_file_search_tools::preparation::prepare_file_search_call;
use crate::api::responses_file_search_tools::synthesis::{
    OriginalModel, OriginalResponse, OutputItems, synthesize_response,
};
use crate::api::responses_file_search_tools::tools::replace_file_search_tools;
use crate::api::responses_tool_namespaces::values::Fields;
use serde_json::Error as JsonError;

use self::decoder::file_search_calls;
use self::error::{FileSearchLoopError, widen};
use self::follow_up::build_follow_up_input;
use self::ports::{ResponsesFollowUp, VectorStoreSearches};
use self::records::{
    ClockSource, ExecutedToolCalls, FileSearchDiagnostic, FollowUpCall, IdSource,
    SearchAuthorization, UpstreamSearch,
};
use self::search::{ToolCallRequest, execute_file_search_tool_calls};

const CALL_ID_PREFIX: &str = "fs_";
const CALL_ID_LENGTH: usize = 24;

// what aresponses_with_emulated_file_search (L515-L629) receives from its caller
pub struct ResponsesFileSearchLoopInput {
    pub input: Json,
    pub model: String,
    pub tools: Option<Vec<Json>>,
    pub kwargs: Fields,
    pub authorization: SearchAuthorization,
}

// the synthesized response, the provider calls it made, the searches it ran and the warnings it emitted
#[derive(Debug, Clone, PartialEq)]
pub struct ResponsesFileSearchLoopResult {
    pub response: ResponsesApiResponse,
    pub hidden_params: HiddenParams,
    pub calls: Vec<FollowUpCall>,
    pub searches: Vec<UpstreamSearch>,
    pub diagnostics: Vec<FileSearchDiagnostic>,
}

pub struct ResponsesFileSearchLoop<'deps, FollowUp, Searches> {
    pub follow_up: &'deps FollowUp,
    pub searches: &'deps Searches,
    pub limits: ApiLimits,
    pub ids: IdSource,
    pub clock: ClockSource,
}

// the first provider call and the request state its result decides the rest of the flow with
struct FirstRound {
    include_search_results: bool,
    vector_store_ids: Vec<String>,
    call: FollowUpCall,
    response: ResponsesApiResponse,
}

// what the round reports next to the synthesized response
struct RoundProgress {
    calls: Vec<FollowUpCall>,
    searches: Vec<UpstreamSearch>,
    diagnostics: Vec<FileSearchDiagnostic>,
}

// ast-grep-ignore: no-long-names - json.loads raw_args at emulated_handler.py:455, name from the task
pub fn decode_responses_file_search_args(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Fields, DecodeError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let value: Json = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(DecodeError::NotObject)
        }
    }
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the file search call arguments are not valid JSON")]
    Json(#[source] JsonError),
    #[error("the file search call arguments are not a JSON object")]
    NotObject,
}

impl<FollowUp, Searches> ResponsesFileSearchLoop<'_, FollowUp, Searches>
where
    FollowUp: ResponsesFollowUp,
    Searches: VectorStoreSearches,
{
    pub async fn run(
        &self,
        input: &ResponsesFileSearchLoopInput,
    ) -> Result<ResponsesFileSearchLoopResult, FileSearchLoopError<FollowUp::Error>> {
        let prepared = prepare_file_search_call(&input.kwargs)?;
        let (tools, vector_store_ids) = replace_file_search_tools(input.tools.as_deref())?;
        let first_call = FollowUpCall {
            input: input.input.clone(),
            model: input.model.clone(),
            tools: (!tools.is_empty()).then_some(tools),
            kwargs: prepared.updated_kwargs,
            internal: true,
        };
        let first_response = self
            .follow_up
            .call(first_call.request())
            .await
            .map_err(FileSearchLoopError::FirstCall)?;
        let tool_calls = file_search_calls(&first_response.output).map_err(widen)?;
        let round = FirstRound {
            include_search_results: prepared.include_search_results,
            vector_store_ids,
            call: first_call,
            response: first_response,
        };
        if tool_calls.is_empty() {
            return self.answer_without_search(input, round);
        }
        self.answer_with_search(input, round, &tool_calls).await
    }

    fn answer_without_search(
        &self,
        input: &ResponsesFileSearchLoopInput,
        round: FirstRound,
    ) -> Result<ResponsesFileSearchLoopResult, FileSearchLoopError<FollowUp::Error>> {
        let call_id = self.next_call_id();
        let text = response_output_text(&round.response.output)?;
        let outputs = OutputItems {
            call: build_file_search_call_output(
                &call_id,
                &[string_value(&input.input)],
                None,
                false,
            )?,
            message: build_message_output(&text, &[]),
        };
        let progress = RoundProgress {
            calls: vec![round.call],
            searches: Vec::new(),
            diagnostics: Vec::new(),
        };
        self.finish(&round.response, &outputs, progress)
    }

    async fn answer_with_search(
        &self,
        input: &ResponsesFileSearchLoopInput,
        round: FirstRound,
        tool_calls: &[Fields],
    ) -> Result<ResponsesFileSearchLoopResult, FileSearchLoopError<FollowUp::Error>> {
        let call_id = self.next_call_id();
        let request = ToolCallRequest {
            tool_calls,
            vector_store_ids: &round.vector_store_ids,
            input: &input.input,
            fallback_call_id: &call_id,
            limits: &self.limits,
            authorization: &input.authorization,
        };
        let executed = execute_file_search_tool_calls(self.searches, &request)
            .await
            .map_err(widen)?;
        let call = self.follow_up_call(input, &round, &executed)?;
        let final_response = self
            .follow_up
            .call(call.request())
            .await
            .map_err(FileSearchLoopError::FollowUp)?;
        let outputs = Self::final_outputs(
            input,
            &executed,
            &final_response,
            round.include_search_results,
        )?;
        let progress = RoundProgress {
            calls: vec![round.call, call],
            searches: executed.searches,
            diagnostics: executed.diagnostics,
        };
        self.finish(&final_response, &outputs, progress)
    }

    fn finish(
        &self,
        provider_response: &ResponsesApiResponse,
        outputs: &OutputItems,
        progress: RoundProgress,
    ) -> Result<ResponsesFileSearchLoopResult, FileSearchLoopError<FollowUp::Error>> {
        let synthesized = synthesize_response(
            &original_response(provider_response),
            outputs,
            None,
            || (self.ids)(),
            || (self.clock)(),
        )?;
        Ok(ResponsesFileSearchLoopResult {
            response: synthesized.response,
            hidden_params: synthesized.hidden_params,
            calls: progress.calls,
            searches: progress.searches,
            diagnostics: progress.diagnostics,
        })
    }

    // _build_follow_up_input (L489-L512) and the second provider call
    fn follow_up_call(
        &self,
        input: &ResponsesFileSearchLoopInput,
        round: &FirstRound,
        executed: &ExecutedToolCalls,
    ) -> Result<FollowUpCall, FileSearchLoopError<FollowUp::Error>> {
        let follow_up_input = build_follow_up_input(
            &input.input,
            &round.response.output,
            &executed.outputs,
            self.limits,
        )
        .map_err(widen)?;
        Ok(FollowUpCall {
            input: follow_up_input,
            model: input.model.clone(),
            tools: None,
            kwargs: round.call.kwargs.clone(),
            internal: true,
        })
    }

    fn final_outputs(
        input: &ResponsesFileSearchLoopInput,
        executed: &ExecutedToolCalls,
        final_response: &ResponsesApiResponse,
        include_search_results: bool,
    ) -> Result<OutputItems, FileSearchLoopError<FollowUp::Error>> {
        let text = response_output_text(&final_response.output)?;
        let queries = if executed.queries.is_empty() {
            vec![string_value(&input.input)]
        } else {
            executed.queries.clone()
        };
        Ok(OutputItems {
            call: build_file_search_call_output(
                &executed.call_id,
                &queries,
                Some(&executed.results),
                include_search_results,
            )?,
            message: build_message_output(&text, &executed.results),
        })
    }

    fn next_call_id(&self) -> String {
        let generated = (self.ids)();
        let short: String = generated.chars().take(CALL_ID_LENGTH).collect();
        format!("{CALL_ID_PREFIX}{short}")
    }
}

// the fields _synthesize_responses_api_response (L345-L384) copies off the provider's response
fn original_response(response: &ResponsesApiResponse) -> OriginalResponse {
    OriginalResponse {
        id: Some(response.id.clone()),
        created_at: Some(response.created_at),
        model: response.model.as_ref().map_or(OriginalModel::Null, |text| {
            OriginalModel::Text(text.clone())
        }),
        usage: response.usage.clone(),
        hidden_params: None,
    }
}
