use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::{CompletionLogger, PrePostLogger};
use indexmap::IndexMap;
use serde::Serialize;

// serde moves between the wire JSON and the logging tree; a value the tree cannot hold is logged as null
#[must_use]
pub fn logged_json<T: Serialize>(value: &T) -> Json {
    serde_json::from_value(serde_json::to_value(value).unwrap_or_default()).unwrap_or(Json::Null)
}

// main.py's post_call carries the messages, the reply it logged and the resolved supplied headers
pub struct ChatPostCall {
    pub input: Json,
    pub original_response: Json,
    pub headers: Option<IndexMap<String, Json>>,
}

// the text branch logs through the same record, with the reply already rendered to JSON
pub struct TextPostCall {
    pub input: Json,
    pub original_response: Json,
    pub headers: Option<IndexMap<String, Json>>,
}

impl TextPostCall {
    #[must_use]
    pub const fn new(
        input: Json,
        original_response: Json,
        headers: Option<IndexMap<String, Json>>,
    ) -> Self {
        Self {
            input,
            original_response,
            headers,
        }
    }
}

// The two main.py hooks plus the completion logger B2238's transport reads, as one port a fixture can record.
pub trait OpenAiLogger {
    fn chat_post_call(&mut self, record: ChatPostCall);

    fn text_post_call(&mut self, record: TextPostCall);

    fn completion(&mut self) -> &mut dyn CompletionLogger;
}

// B249's logging state and environment, serving the dispatch hooks and the text transport at once
pub struct B249Logger<'call, 'env> {
    pub completion: PrePostLogger<'call, 'env>,
}

impl OpenAiLogger for B249Logger<'_, '_> {
    fn chat_post_call(&mut self, record: ChatPostCall) {
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

    fn text_post_call(&mut self, record: TextPostCall) {
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
const fn headers_only(headers: Option<IndexMap<String, Json>>) -> AdditionalArgs {
    AdditionalArgs {
        api_base: None,
        headers,
        complete_input_dict: None,
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}
