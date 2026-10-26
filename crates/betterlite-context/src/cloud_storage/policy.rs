use std::fmt;
#[derive(Clone, PartialEq, Eq)]
pub enum LegacyCloudSetting {
    Boolean(bool),
    Text(String),
    Other,
}
pub struct TrustedCloudCredentials {
    setting: LegacyCloudSetting,
}
impl From<LegacyCloudSetting> for TrustedCloudCredentials {
    fn from(value: LegacyCloudSetting) -> Self {
        Self { setting: value }
    }
}
impl fmt::Debug for TrustedCloudCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrustedCloudCredentials")
    }
}
#[must_use]
pub fn allow_legacy(params: Option<&TrustedCloudCredentials>) -> bool {
    match params.map(|credentials| &credentials.setting) {
        Some(LegacyCloudSetting::Boolean(value)) => *value,
        Some(LegacyCloudSetting::Text(value)) => matches!(
            value
                .trim_matches(|character: char| character.is_whitespace()
                    || ('\u{1c}'..='\u{1f}').contains(&character))
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Some(LegacyCloudSetting::Other) | None => false,
    }
}
