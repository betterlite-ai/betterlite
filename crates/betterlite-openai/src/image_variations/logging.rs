use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::ImageVariationData;
use crate::api::image_contracts::records::ImageHeaders;

pub trait ImageVariationLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;

    fn post_call(&mut self, record: PostCallRecord);
}

pub struct PreCallRecord {
    pub headers: Option<ImageHeaders>,
    pub api_base: String,
    pub data: ImageVariationData,
}

pub struct PostCallRecord {
    pub headers: Option<ImageHeaders>,
    pub api_base: String,
    pub original_response: Json,
}

// the image is bytes, so a log record names its upload instead of carrying it
#[must_use]
pub fn data_view(data: &ImageVariationData) -> IndexMap<String, Json> {
    let image = IndexMap::from([
        (
            String::from("filename"),
            data.image.filename.clone().map_or(Json::Null, Json::Text),
        ),
        (
            String::from("content_type"),
            data.image
                .content_type
                .clone()
                .map_or(Json::Null, Json::Text),
        ),
        (
            String::from("size"),
            Json::Number(serde_json::Number::from(data.image.content.len())),
        ),
    ]);
    let mut inner = IndexMap::from([(String::from("image"), Json::Object(image))]);
    inner.extend(data.options.clone());
    IndexMap::from([(String::from("data"), Json::Object(inner))])
}

fn headers_view(headers: Option<&ImageHeaders>) -> Option<IndexMap<String, Json>> {
    let given = headers?;
    Some(
        given
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

pub struct PrePostLogger<'call, 'env> {
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

impl ImageVariationLogger for PrePostLogger<'_, '_> {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        pre_call(
            self.state,
            &PreCall {
                input: Json::Text(String::new()),
                model: None,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base),
                    headers: headers_view(record.headers.as_ref()),
                    complete_input_dict: Some(RawBody::Object(data_view(&record.data))),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        )
        .map(drop)
    }

    fn post_call(&mut self, record: PostCallRecord) {
        post_call(
            self.state,
            &PostCall {
                input: Json::Null,
                original_response: record.original_response,
                additional_args: AdditionalArgs {
                    api_base: Some(record.api_base),
                    headers: headers_view(record.headers.as_ref()),
                    complete_input_dict: None,
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            self.env,
        );
    }
}
