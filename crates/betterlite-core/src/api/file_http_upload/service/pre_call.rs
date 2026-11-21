use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileCreate;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::file_http_upload::config::Headers;
use crate::api::file_http_upload::error::UploadError;
use crate::api::file_http_upload::plan::UploadPlan;

use super::LogContext;

pub(super) fn caller_headers(data: &FileCreate) -> Headers {
    data.extra_headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

// create_file's async pre_call, with the transformed request masked and no request string
pub(super) fn log_pre_call(
    api_base: &str,
    headers: &Headers,
    plan: &UploadPlan,
    log: &mut LogContext<'_, '_>,
) -> Result<(), UploadError> {
    let logged = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.expose_secret().to_owned())))
        .collect();
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: plan.logged_input().map(raw_body),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)?;
    // LiteLLM passes api_key="" beside input, and B249's pre_call record has no member to carry it
    log.state
        .model_call_details
        .insert("api_key".to_owned(), Json::Text(String::new()));
    Ok(())
}

// a plan logs an object or a placeholder text; any other JSON keeps the shape it was built with
fn raw_body(value: Json) -> RawBody {
    match value {
        Json::Object(fields) => RawBody::Object(fields),
        Json::Text(text) => RawBody::Text(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            RawBody::Text(serde_json::to_string(&value).unwrap_or_default())
        }
    }
}
