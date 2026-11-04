use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;

#[derive(Debug, Default, Deserialize)]
pub struct CallbackSettings {
    callbacks: Option<CallbackSetting>,
    success_callback: Option<CallbackSetting>,
    failure_callback: Option<CallbackSetting>,
    #[serde(flatten)]
    _other_settings: BTreeMap<String, CallbackSetting>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CallbackSetting {
    String(String),
    List(Vec<CallbackEntry>),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CallbackEntry {
    String(String),
    Other(IgnoredAny),
}

impl CallbackSettings {
    pub fn is_configured(&self) -> bool {
        match (
            &self.callbacks,
            &self.success_callback,
            &self.failure_callback,
        ) {
            (Some(callbacks), success, failure) => {
                callbacks.matches()
                    || success.as_ref().is_some_and(CallbackSetting::matches)
                    || failure.as_ref().is_some_and(CallbackSetting::matches)
            }
            (None, success, failure) => {
                success.as_ref().is_some_and(CallbackSetting::matches)
                    || failure.as_ref().is_some_and(CallbackSetting::matches)
            }
        }
    }
}

impl CallbackSetting {
    fn matches(&self) -> bool {
        match self {
            Self::String(value) => value == "prometheus",
            Self::List(values) => values.iter().any(CallbackEntry::matches),
            Self::Other(_) => false,
        }
    }
}

impl CallbackEntry {
    fn matches(&self) -> bool {
        match self {
            Self::String(value) => value == "prometheus",
            Self::Other(_) => false,
        }
    }
}
