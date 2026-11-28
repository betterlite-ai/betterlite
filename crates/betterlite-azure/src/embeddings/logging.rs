use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::client::Fields;

pub struct PreCallRecord {
    pub input: Json,
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub complete_input_dict: Fields,
}

// the sync path logs the SDK object, the async path its dumped dict, and a failed async call str(error)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    SdkObject,
    Dumped,
    ErrorText,
}

pub struct PostCallRecord {
    pub input: Json,
    pub api_key: Option<SecretString>,
    pub original_response: Json,
    pub kind: ResponseKind,
    pub complete_input_dict: Fields,
    pub api_base: Option<String>,
}

pub trait EmbeddingLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;

    fn post_call(&mut self, record: PostCallRecord);
}

fn additional_args(
    headers: Option<IndexMap<String, Json>>,
    api_base: Option<String>,
    complete_input_dict: Fields,
) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers,
        complete_input_dict: Some(RawBody::Object(complete_input_dict)),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

impl EmbeddingLogger for LogContext<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        let secret = |key: Option<&SecretString>| {
            key.map_or(Json::Null, |secret| {
                Json::Text(secret.expose_secret().to_owned())
            })
        };
        let headers = IndexMap::from([
            ("api_key".to_owned(), secret(record.api_key.as_ref())),
            (
                "azure_ad_token".to_owned(),
                secret(record.azure_ad_token.as_ref()),
            ),
        ]);
        let call = PreCall {
            input: record.input,
            model: None,
            additional_args: additional_args(Some(headers), None, record.complete_input_dict),
        };
        pre_call(self.state, &call, self.env)?;
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        let call = PostCall {
            original_response: record.original_response,
            input: record.input,
            additional_args: additional_args(None, record.api_base, record.complete_input_dict),
        };
        post_call(self.state, &call, self.env);
    }
}
