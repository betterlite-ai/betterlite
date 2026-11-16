use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
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

// B249's pre-call and post-call dispatch over the handler's records
pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
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

impl EmbeddingLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        let headers = IndexMap::from([
            (
                "api_key".to_owned(),
                record
                    .api_key
                    .as_ref()
                    .map_or(Json::Null, |key| Json::Text(key.expose_secret().to_owned())),
            ),
            (
                "azure_ad_token".to_owned(),
                record.azure_ad_token.as_ref().map_or(Json::Null, |token| {
                    Json::Text(token.expose_secret().to_owned())
                }),
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
