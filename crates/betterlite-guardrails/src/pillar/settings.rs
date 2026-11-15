use std::time::Duration;

use secrecy::SecretString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFlaggedAction {
    Block,
    Monitor,
    Mask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackAction {
    Allow,
    Block,
}

impl OnFlaggedAction {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "block" => Some(Self::Block),
            "monitor" => Some(Self::Monitor),
            "mask" => Some(Self::Mask),
            _ => None,
        }
    }
}

impl FallbackAction {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "allow" => Some(Self::Allow),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PillarSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
    pub on_flagged_action: OnFlaggedAction,
    pub async_mode: Option<bool>,
    pub persist_session: Option<bool>,
    pub include_scanners: bool,
    pub include_evidence: bool,
    pub fallback_on_error: FallbackAction,
    pub timeout: Duration,
}
