use serde::Deserialize;

use super::HashicorpSecretManager;
use super::settings::DEFAULT_MOUNT;
use super::transport::VaultTransport;

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

impl VaultSecretSettings {
    pub(super) fn data_key(&self) -> String {
        let trimmed = self.data.as_deref().map(str::trim);
        trimmed
            .filter(|key| !key.is_empty())
            .unwrap_or("key")
            .to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSecretTarget {
    pub url: String,
    pub data_key: String,
    pub secret_name: String,
}

fn sanitize_path_component(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim().trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

pub(super) fn secret_url<T>(
    manager: &HashicorpSecretManager<T>,
    secret_name: &str,
    settings: &VaultSecretSettings,
) -> String
where
    T: VaultTransport,
{
    let manager_namespace = manager.vault_secret_namespace();
    let namespace_source = settings.namespace.as_deref().or(manager_namespace);
    let namespace = sanitize_path_component(namespace_source);
    let mount_source = settings
        .mount
        .as_deref()
        .unwrap_or(manager.settings.mount.as_str());
    let mount =
        sanitize_path_component(Some(mount_source)).unwrap_or_else(|| DEFAULT_MOUNT.to_owned());
    let prefix_source = settings
        .path_prefix
        .as_deref()
        .or(manager.settings.path_prefix.as_deref());
    let prefix = sanitize_path_component(prefix_source);
    let mut url = format!("{}/v1/", manager.vault_addr());
    if let Some(segment) = namespace {
        url.push_str(&segment);
        url.push('/');
    }
    url.push_str(&mount);
    url.push_str("/data/");
    if let Some(segment) = prefix {
        url.push_str(&segment);
        url.push('/');
    }
    url.push_str(secret_name);
    url
}
