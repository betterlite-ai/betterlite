use serde::Deserialize;

mod tests;

#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
pub struct VaultSecretSettings {
    pub namespace: Option<String>,
    pub mount: Option<String>,
    pub path_prefix: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
pub struct VaultSecretParams {
    pub secret_manager_settings: Option<VaultSecretSettings>,
    #[serde(flatten)]
    pub flat: VaultSecretSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSecretTarget {
    pub url: String,
    pub data_key: String,
    pub secret_name: String,
}

pub(super) fn sanitize_path_component(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim().trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}
