use betterlite_azure::azure_stream_errors::error::StreamingError;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::HelperError;

use super::super::hooks::Recorder;
use super::super::source::ScriptedSource;
use super::super::source::credentials::ScriptedSources;
use super::super::source::requests;
use super::{error_value, exchanges, failure, gets, pre_value, response_headers};

pub(crate) struct Outcome {
    pub(crate) name: &'static str,
    pub(crate) wrapper: Json,
    pub(crate) chunks: Json,
    pub(crate) error: Json,
}

pub(crate) fn error_report(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    recorder: &Recorder,
    error: &StreamingError,
) -> Result<Json, Box<HelperError>> {
    match error {
        StreamingError::Azure(raised) => Ok(report(
            source,
            sources,
            recorder,
            Outcome {
                name: "error",
                wrapper: Json::Null,
                chunks: Json::Null,
                error: error_value(raised),
            },
        )),
        StreamingError::Logging(_) | StreamingError::Chunk(_) | StreamingError::Usage(_) => {
            Err(failure(error))
        }
    }
}

pub(crate) fn report(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    recorder: &Recorder,
    outcome: Outcome,
) -> Json {
    let mut fields = Extra::new();
    fields.insert("outcome".to_owned(), Json::Text(outcome.name.to_owned()));
    fields.insert("gets".to_owned(), Json::List(gets(source)));
    fields.insert("requests".to_owned(), Json::List(requests(source)));
    fields.insert("exchanges".to_owned(), Json::List(exchanges(sources)));
    fields.insert(
        "pre".to_owned(),
        Json::List(recorder.pre.iter().map(pre_value).collect()),
    );
    fields.insert("response_headers".to_owned(), response_headers(recorder));
    fields.insert("wrapper".to_owned(), outcome.wrapper);
    fields.insert("chunks".to_owned(), outcome.chunks);
    fields.insert("error".to_owned(), outcome.error);
    Json::Object(fields)
}
