use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::CompletionLogger;
use indexmap::IndexMap;
use serde::Serialize;

// serde moves between the wire JSON and the logging tree; a value the tree cannot hold is logged as null
#[must_use]
pub fn logged_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    let Ok(serialized) = serde_json::to_value(value) else {
        return Json::Null;
    };
    serde_json::from_value(serialized).unwrap_or(Json::Null)
}

// main.py's post_call carries the messages, the reply or the error text it logged, and the headers it resolved
pub struct LoggedCall {
    pub input: Json,
    pub original_response: Json,
    pub headers: Option<IndexMap<String, Json>>,
}

// The main.py hook plus the completion logger B2238's transport reads, as one port a fixture can record.
pub trait OpenAiLogger {
    fn post_call(&mut self, record: LoggedCall);

    fn completion(&mut self) -> &mut dyn CompletionLogger;
}

// B249's logging state and environment, serving the dispatch hooks and the text transport at once
pub struct B249Logger<'call, 'env> {
    pub completion: LogContext<'call, 'env>,
}

impl OpenAiLogger for B249Logger<'_, '_> {
    fn post_call(&mut self, record: LoggedCall) {
        post_call(
            self.completion.state,
            &PostCall {
                original_response: record.original_response,
                input: record.input,
                additional_args: headers_only(record.headers),
            },
            self.completion.env,
        );
    }

    fn completion(&mut self) -> &mut dyn CompletionLogger {
        &mut self.completion
    }
}

// main.py's additional_args names the headers alone; the base and the input dictionary stay unrecorded there
fn headers_only(headers: Option<IndexMap<String, Json>>) -> AdditionalArgs {
    AdditionalArgs {
        api_base: None,
        headers,
        complete_input_dict: None,
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}
