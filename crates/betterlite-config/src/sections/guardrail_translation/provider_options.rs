use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LakeraOnFlagged {
    Block,
    Monitor,
    InjectSystemMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LassoGuardrailConfigModel {
    #[serde(default)]
    pub lasso_user_id: Option<String>,
    #[serde(default)]
    pub lasso_conversation_id: Option<String>,
    #[serde(default = "default_mask")]
    pub mask: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepKeepGuardrailConfigModel {
    pub deepkeep_firewall_id: Option<String>,
}

// LiteLLM declares these `bool | None` with a non-None default, so an absent key takes the value and null stays None
#[must_use]
pub const fn default_payload() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_breakdown() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_dev_info() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_on_flagged() -> Option<LakeraOnFlagged> {
    Some(LakeraOnFlagged::Block)
}

#[must_use]
pub const fn default_mask() -> Option<bool> {
    Some(false)
}

#[must_use]
pub fn default_on_flagged_action() -> Option<String> {
    Some("monitor".to_owned())
}

#[must_use]
pub const fn default_include_scanners() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_include_evidence() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_use_v2() -> Option<bool> {
    Some(false)
}

#[must_use]
pub const fn default_send_user_alias() -> Option<bool> {
    Some(false)
}

#[must_use]
pub const fn default_send_user_id() -> Option<bool> {
    Some(false)
}

#[must_use]
pub const fn default_send_team_id() -> Option<bool> {
    Some(false)
}
