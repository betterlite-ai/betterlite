use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::serde::arbitrary_precision_option;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{As, Same};

use crate::spend_bodies::session::SpendTimestamp;
use crate::spend_event::SPEND_EVENT_VERSION;
use crate::spend_event::response::CompactResponse;

pub type Mapping = IndexMap<String, Json>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventVersion;

impl Serialize for EventVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(SPEND_EVENT_VERSION)
    }
}

impl<'de> Deserialize<'de> for EventVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = u8::deserialize(deserializer)?;
        if version == SPEND_EVENT_VERSION {
            Ok(Self)
        } else {
            Err(D::Error::custom(format!(
                "unsupported spend event version {version}"
            )))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct LitellmParams {
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub litellm_call_id: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub metadata: Option<Mapping>,
    pub litellm_metadata: Option<Mapping>,
    pub proxy_server_request: Option<Mapping>,
    pub preset_cache_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DynamicParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_off_message_logging: Option<bool>,
}

// Every field is required on the wire, and a nullable one must still be written as null
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpendEvent {
    pub version: EventVersion,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub litellm_call_id: Option<String>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub call_type: Option<String>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub model: Option<String>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub custom_llm_provider: Option<String>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub stream: Option<bool>,
    pub complete_streaming_response: bool,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub cache_hit: Option<bool>,
    #[serde(with = "arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    pub start_time: SpendTimestamp,
    pub end_time: SpendTimestamp,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub completion_start_time: Option<SpendTimestamp>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub agent_id: Option<String>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub litellm_trace_id: Option<String>,
    pub litellm_params: LitellmParams,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub standard_logging_object: Option<Mapping>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub standard_callback_dynamic_params: Option<DynamicParams>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub response: Option<CompactResponse>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub combined_usage: Option<Mapping>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub realtime_tools: Option<Vec<Json>>,
    #[serde(deserialize_with = "As::<Option<Same>>::deserialize")]
    pub realtime_tool_calls: Option<Vec<Json>>,
    pub request_tool_names: Vec<String>,
}
