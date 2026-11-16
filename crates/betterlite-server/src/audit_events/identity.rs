use betterlite_config::sections::settings::JsonValue;
use secrecy::SecretString;

pub const CHANGED_BY_HEADER_METADATA_KEY: &str = "allow_litellm_changed_by_header";

#[derive(Clone, Debug, Default)]
pub struct CallerIdentity {
    pub user_id: Option<String>,
    pub api_key: Option<SecretString>,
    pub metadata: Option<JsonValue>,
    pub team_metadata: Option<JsonValue>,
}

impl CallerIdentity {
    fn allows_changed_by_header(&self) -> bool {
        [self.metadata.as_ref(), self.team_metadata.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|metadata| metadata.get(CHANGED_BY_HEADER_METADATA_KEY))
            .any(|value| value.as_bool() == Some(true))
    }
}

#[must_use]
pub fn get_audit_log_changed_by(
    litellm_changed_by: Option<&str>,
    caller: &CallerIdentity,
    litellm_proxy_admin_name: Option<&str>,
) -> Option<String> {
    let header = litellm_changed_by.filter(|value| !value.is_empty());
    if header.is_some() && caller.allows_changed_by_header() {
        return header.map(str::to_owned);
    }
    let user_id = caller.user_id.clone().filter(|value| !value.is_empty());
    user_id.or_else(|| litellm_proxy_admin_name.map(str::to_owned))
}
