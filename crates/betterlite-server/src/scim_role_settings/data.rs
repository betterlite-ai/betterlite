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
    pub active: Option<bool>,
    pub enterprise: Option<EnterpriseUser>,
    pub entitlements: Option<Vec<DirectoryAttribute>>,
    pub roles: Option<Vec<DirectoryAttribute>>,
}

#[must_use]
pub fn extract_scim_user_data(user: &User) -> ScimUserData {
    let scim_name = user.name.as_ref();
    ScimUserData {
        user_email: match user.emails.as_deref() {
            Some([first, ..]) => Some(first.value.0.clone()),
            _ => None,
        },
        user_alias: match scim_name.and_then(|name| name.given_name.as_deref()) {
            Some(given) if !given.is_empty() => Some(given.to_owned()),
            _ => None,
        },
        sso_user_id: user.external_id.clone(),
        teams: match user.groups.as_ref() {
            Some(groups) => groups.iter().map(|group| group.value.clone()).collect(),
            None => Vec::new(),
        },
        given_name: scim_name.and_then(|name| name.given_name.clone()),
        family_name: scim_name.and_then(|name| name.family_name.clone()),
        // the SCIMUser model defaults active to true, so an extraction always knows the value
        active: Some(active_value(user.active)),
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
