use crate::scim_records::attribute::DirectoryAttribute;
use crate::scim_records::user::{Active, EnterpriseUser, User};

// the fields POST and PUT share, read the way _extract_scim_user_data reads them
#[derive(Debug, Clone)]
pub struct ScimUserData {
    pub user_email: Option<String>,
    pub user_alias: Option<String>,
    pub sso_user_id: Option<String>,
    pub teams: Vec<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub active: bool,
    pub enterprise: Option<EnterpriseUser>,
    pub entitlements: Option<Vec<DirectoryAttribute>>,
    pub roles: Option<Vec<DirectoryAttribute>>,
}

#[must_use]
pub fn extract_scim_user_data(user: &User) -> ScimUserData {
    let scim_name = user.name.as_ref();
    let emails = user.emails.as_deref().unwrap_or_default();
    ScimUserData {
        user_email: emails.first().map(|email| email.value.0.clone()),
        user_alias: scim_name
            .and_then(|name| name.given_name.as_deref())
            .filter(|given| !given.is_empty())
            .map(str::to_owned),
        sso_user_id: user.external_id.clone(),
        teams: user
            .groups
            .iter()
            .flatten()
            .map(|group| group.value.clone())
            .collect(),
        given_name: scim_name.and_then(|name| name.given_name.clone()),
        family_name: scim_name.and_then(|name| name.family_name.clone()),
        // the SCIMUser model defaults active to true, so an extraction always knows the value
        active: active_value(user.active),
        enterprise: user.enterprise.clone(),
        entitlements: user.entitlements.clone(),
        roles: user.roles.clone(),
    }
}

const fn active_value(active: Active) -> bool {
    match active {
        Active::Sent(flag) | Active::Defaulted(flag) => flag.0,
    }
}
