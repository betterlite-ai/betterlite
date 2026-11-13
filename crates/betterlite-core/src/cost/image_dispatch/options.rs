use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use regex::Regex;

use super::error::ImageDispatchError;

pub type OptionalParams = IndexMap<String, Json>;

// A string-typed optional parameter, or None for a missing key or a value of any other type.
#[must_use]
pub fn requested_image_param<'params>(
    optional_params: Option<&'params OptionalParams>,
    key: &str,
) -> Option<&'params str> {
    match optional_params?.get(key)? {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// The compiled `\d+(?:x|-x-)\d+` full-match pattern; \d takes every Unicode decimal digit like Python's str pattern.
pub struct SizePattern(Regex);

impl SizePattern {
    pub fn new() -> Result<Self, ImageDispatchError> {
        let pattern = Regex::new(r"\A\d+(?:x|-x-)\d+\z")
            .map_err(|source| ImageDispatchError::Pattern { source })?;
        Ok(Self(pattern))
    }

    #[must_use]
    pub fn requested_size<'params>(
        &self,
        optional_params: Option<&'params OptionalParams>,
    ) -> Option<&'params str> {
        requested_image_param(optional_params, "size")
            .and_then(|size| self.0.is_match(size).then_some(size))
    }
}

// The call types whose response is an ImageResponse; the passthrough one has no CallType variant.
#[must_use]
pub fn call_type_has_image_response(call_type: &str) -> bool {
    const IMAGE_CALLS: [&str; 5] = [
        "image_generation",
        "aimage_generation",
        "passthrough_image_generation",
        "image_edit",
        "aimage_edit",
    ];
    IMAGE_CALLS.contains(&call_type)
}
