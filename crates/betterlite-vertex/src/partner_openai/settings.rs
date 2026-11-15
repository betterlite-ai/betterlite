use serde::{Deserialize, Serialize};

// The AI21 and Llama classes put their constructor max_tokens on the class, and get_config copies
// every non-None class attribute, so that attribute is the whole typed snapshot for both partners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
}
