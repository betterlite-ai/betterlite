use serde::{Deserialize, Serialize};

// get_config copies each non-None class attribute; the constructor max_tokens is the whole snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
}
