use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

#[derive(serde::Deserialize)]
pub(crate) struct MinimumCustomKeyLength(usize);

impl Default for MinimumCustomKeyLength {
    fn default() -> Self {
        Self(MINIMUM_CUSTOM_KEY_LENGTH)
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "redact_string")]
    Text {
        text: String,
        #[serde(default)]
        minimum_custom_key_length: MinimumCustomKeyLength,
    },
    #[serde(rename = "redact_internal_details")]
    InternalDetails {
        text: String,
        #[serde(default)]
        minimum_custom_key_length: MinimumCustomKeyLength,
    },
    #[serde(rename = "redact_structured_value")]
    Structured {
        #[serde(default)]
        key: Option<String>,
        value: String,
        #[serde(default)]
        minimum_custom_key_length: MinimumCustomKeyLength,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let minimum_custom_key_length = match input {
            Input::Text {
                minimum_custom_key_length,
                ..
            }
            | Input::InternalDetails {
                minimum_custom_key_length,
                ..
            }
            | Input::Structured {
                minimum_custom_key_length,
                ..
            } => minimum_custom_key_length.0,
        };
        let redactor = Redactor::new(minimum_custom_key_length).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "PatternError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let result = match input {
            Input::Text { text, .. } => redactor.redact_string(text),
            Input::InternalDetails { text, .. } => redactor.redact_internal_details(text),
            Input::Structured { key, value, .. } => {
                redactor.redact_structured_value(key.as_deref(), value)
            }
        };
        Ok(serde_json::json!({ "result": result }))
    }
}

mod tests;
