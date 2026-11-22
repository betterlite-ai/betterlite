use std::collections::BTreeMap;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::{
    CreateCharacterCall, DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, GetCharacterCall,
};
use betterlite_core::api::video_records::CharacterObject;
use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::CharacterKind;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer};

const CHARACTER_ID: &str = "char-1";
const CHARACTER_CREATED_AT: i64 = 1_700_000_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) function: Function,
    pub(super) asynchronous: bool,
    litellm_call_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    video: Option<String>,
    #[serde(default)]
    character_id: Option<String>,
    // absent keeps the declared 600, null falls back to REQUEST_TIMEOUT and a number is the caller's
    #[serde(default, deserialize_with = "declared_timeout")]
    timeout: Option<Option<f64>>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    extra: IndexMap<String, Json>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    Create,
    Get,
}

fn declared_timeout<'de, D>(deserializer: D) -> Result<Option<Option<f64>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<f64>::deserialize(deserializer).map(Some)
}

impl Input {
    fn provider(&self) -> String {
        self.custom_llm_provider
            .clone()
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
    }

    pub(super) fn create_call(&self) -> CreateCharacterCall {
        CreateCharacterCall {
            name: self.name.clone().unwrap_or_default(),
            video: SuppliedMedia::Text(self.video.clone().unwrap_or_default()),
            timeout: self.timeout.unwrap_or(Some(DEFAULT_TIMEOUT_SECONDS)),
            custom_llm_provider: self.provider(),
            extra_headers: self.extra_headers.clone(),
            litellm_call_id: Some(self.litellm_call_id.clone()),
            extra: self.extra.clone(),
            ..CreateCharacterCall::default()
        }
    }

    pub(super) fn get_call(&self) -> GetCharacterCall {
        let mut call = GetCharacterCall {
            character_id: self.character_id.clone().unwrap_or_default(),
            ..GetCharacterCall::default()
        };
        call.timeout = self.timeout.unwrap_or(Some(DEFAULT_TIMEOUT_SECONDS));
        call.custom_llm_provider = self.provider();
        call.extra_headers.clone_from(&self.extra_headers);
        call.litellm_call_id = Some(self.litellm_call_id.clone());
        call.extra.clone_from(&self.extra);
        call
    }

    // the object the fixture handler answers with, named by the create call or the lookup's fixed name
    pub(super) fn answered_object(&self) -> CharacterObject {
        CharacterObject {
            id: CHARACTER_ID.to_owned(),
            object: CharacterKind::Character,
            created_at: CHARACTER_CREATED_AT,
            name: self.answered_name(),
            hidden_params: IndexMap::new(),
        }
    }

    fn answered_name(&self) -> String {
        match self.function {
            Function::Create => self.name.clone().unwrap_or_default(),
            Function::Get => "Wren".to_owned(),
        }
    }
}
