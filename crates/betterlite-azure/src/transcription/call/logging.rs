use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The pre_call audio_transcriptions makes: the synthetic audio_file input, the client's Authorization header, base URL and data dict.
pub struct AzureTranscriptionPreCall<'record> {
    pub input: &'record str,
    pub authorization: &'record str,
    pub api_base: &'record str,
    pub data: IndexMap<String, Json>,
}

pub struct AzureTranscriptionPostCall<'record> {
    pub input: &'record str,
    // the async branch passes its client's headers and base URL again; the sync branch and a failure pass neither
    pub authorization: Option<&'record str>,
    pub api_base: Option<&'record str>,
    pub original_response: Json,
    // the data reaches post_call on success only
    pub data: Option<IndexMap<String, Json>>,
}

pub trait AzureTranscriptionLogger {
    fn pre_call(&mut self, record: AzureTranscriptionPreCall<'_>) -> Result<(), PrePostError>;

    fn post_call(&mut self, record: AzureTranscriptionPostCall<'_>);
}

impl AzureTranscriptionLogger for LogContext<'_, '_> {
    fn pre_call(&mut self, record: AzureTranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        pre_call(
            self.state,
            &PreCall {
                input: Json::Text(record.input.to_owned()),
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base.to_owned()),
                    headers: Some(IndexMap::from([(
                        String::from("Authorization"),
                        Json::Text(record.authorization.to_owned()),
                    )])),
                    complete_input_dict: Some(RawBody::Object(record.data)),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::from([(
                        String::from("atranscription"),
                        Json::Bool(true),
                    )]),
                },
            },
            self.env,
        )
        .map(drop)
    }

    fn post_call(&mut self, record: AzureTranscriptionPostCall<'_>) {
        post_call(
            self.state,
            &PostCall {
                input: Json::Text(record.input.to_owned()),
                original_response: record.original_response,
                additional_args: AdditionalArgs {
                    api_base: record.api_base.map(str::to_owned),
                    headers: record.authorization.map(|authorization| {
                        IndexMap::from([(
                            String::from("Authorization"),
                            Json::Text(authorization.to_owned()),
                        )])
                    }),
                    complete_input_dict: record.data.map(RawBody::Object),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        );
    }
}
