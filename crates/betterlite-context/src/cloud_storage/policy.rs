#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyCloudSetting {
    Boolean(bool),
    Text(String),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedCloudCredentials {
    pub allow_legacy_cloud_file_ids: LegacyCloudSetting,
}

#[must_use]
pub fn allow_legacy(params: Option<&TrustedCloudCredentials>) -> bool {
    let Some(credentials) = params else {
        return false;
    };
    match &credentials.allow_legacy_cloud_file_ids {
        LegacyCloudSetting::Boolean(value) => *value,
        LegacyCloudSetting::Text(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        LegacyCloudSetting::Other => false,
    }
}
