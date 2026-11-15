use betterlite_prompts::arguments::json::Object;
use secrecy::ExposeSecret as _;

use crate::mai_images::error::MaiImageError;
use crate::mai_images::params::map_openai_params;
use crate::mai_images::reply::{ImageLogging, PostCall};

pub(super) const MODEL: &str = "MAI-Image-2.5";

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn mapped(
    non_default: &str,
    optional: &str,
    drop_params: bool,
) -> Result<Object, MaiImageError> {
    map_openai_params(&object(non_default), object(optional), MODEL, drop_params)
}

pub(super) struct RecordedCall {
    pub(super) input: String,
    pub(super) api_key: Option<String>,
    pub(super) complete_input_dict: Object,
    pub(super) original_response: Object,
}

#[derive(Default)]
pub(super) struct RecordedLogging {
    pub(super) calls: Vec<RecordedCall>,
}

impl ImageLogging for RecordedLogging {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.calls.push(RecordedCall {
            input: call.input.to_owned(),
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            complete_input_dict: call.complete_input_dict.clone(),
            original_response: call.original_response.clone(),
        });
    }
}
