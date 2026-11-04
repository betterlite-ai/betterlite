use betterlite_config::sections::logging_privacy::Settings;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{CallbackMetadataError, PrivacyContext, callback_metadata};
use betterlite_models::decisions::Json;

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Flag {
    Boolean(bool),
    Text(String),
}

impl Flag {
    pub fn resolve(&self) -> Result<Option<bool>, CallbackMetadataError> {
        match self {
            Self::Boolean(value) => Ok(Some(*value)),
            Self::Text(text) if text.contains("os.environ/") => {
                Err(CallbackMetadataError::EnvironmentReference {
                    field: "turn_off_message_logging".to_owned(),
                })
            }
            Self::Text(text) => Ok(match text.trim().to_ascii_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }),
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
pub struct Metadata {
    pub turn_off_message_logging: Option<Flag>,
    pub redact_user_api_key_info: Option<bool>,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::DefaultOnNull>::deserialize"
    )]
    pub headers: IndexMap<String, Flag>,
    #[serde(flatten)]
    pub callback_params: IndexMap<String, Json>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Params {
    pub metadata: Option<Metadata>,
}

#[derive(Default, Deserialize, Serialize)]
pub struct PrivacyRequest {
    #[serde(flatten)]
    pub direct: Metadata,
    pub litellm_params: Option<Params>,
    pub litellm_metadata: Option<Metadata>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Copy)]
pub struct Decision {
    pub content: bool,
    pub key_info: bool,
}

impl PrivacyRequest {
    fn slots(&self) -> [Option<&Metadata>; 4] {
        [
            Some(&self.direct),
            self.litellm_params
                .as_ref()
                .and_then(|params| params.metadata.as_ref()),
            self.litellm_metadata.as_ref(),
            self.metadata.as_ref(),
        ]
    }

    pub fn dynamic_params(&self) -> Result<IndexMap<String, Json>, CallbackMetadataError> {
        let slots = self.slots();
        let mut output = IndexMap::new();
        for slot in slots.iter().flatten() {
            let checked = callback_metadata(&Json::Object(slot.callback_params.clone()))?;
            for (name, value) in checked.fields {
                output.entry(name).or_insert(value);
            }
            if let Some(flag) = &slot.turn_off_message_logging
                && let Some(disabled) = flag.resolve()?
            {
                output
                    .entry("turn_off_message_logging".to_owned())
                    .or_insert(Json::Bool(disabled));
            }
        }
        Ok(output)
    }

    pub fn decide(
        &self,
        settings: Settings,
        context: PrivacyContext,
        callback_message_logging: bool,
    ) -> Result<Decision, CallbackMetadataError> {
        let slots = self.slots();
        for slot in slots.iter().flatten() {
            callback_metadata(&Json::Object(slot.callback_params.clone()))?;
        }
        let dynamic = slots
            .iter()
            .flatten()
            .find_map(|slot| slot.turn_off_message_logging.as_ref())
            .map(Flag::resolve)
            .transpose()?
            .flatten();
        let headers = self
            .litellm_metadata
            .as_ref()
            .or(self.metadata.as_ref())
            .unwrap_or(&self.direct);
        let disabled = header(headers, "litellm-disable-message-redaction")?;
        let enabled = header(headers, "litellm-enable-message-redaction")?
            || header(headers, "x-litellm-enable-message-redaction")?;
        Ok(Decision {
            content: context.message_logging_disabled
                || !callback_message_logging
                || (!disabled && dynamic.unwrap_or(enabled || settings.turn_off_message_logging)),
            key_info: settings.redact_user_api_key_info
                || slots
                    .iter()
                    .flatten()
                    .find_map(|slot| slot.redact_user_api_key_info)
                    .unwrap_or(false),
        })
    }
}

fn header(metadata: &Metadata, name: &str) -> Result<bool, CallbackMetadataError> {
    Ok(metadata
        .headers
        .get(name)
        .map(Flag::resolve)
        .transpose()?
        .flatten()
        .unwrap_or(false))
}

pub fn decide_request<Request>(
    request: &Request,
    settings: Settings,
    context: PrivacyContext,
    callback_message_logging: bool,
) -> Result<Decision, CallbackMetadataError>
where
    Request: Serialize,
{
    let encoded = serde_json::to_vec(request)?;
    let privacy: PrivacyRequest = serde_json::from_slice(&encoded)?;
    privacy.decide(settings, context, callback_message_logging)
}
