use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_config::sso_mappings::{SsoMappingSources, load_mapping_sources};
use betterlite_db::auxiliary_repository::repository::TableError;

use crate::sso_mappings::repository::SsoSettingsRows;
use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

pub(crate) fn sources(pairs: &[(&str, &str)]) -> SsoMappingSources {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    load_mapping_sources(&environment)
}

pub(crate) fn role_mapping(
    group_claim: &str,
    default_role: Option<UserRole>,
    roles: Vec<(UserRole, Vec<&str>)>,
) -> RoleMappings {
    RoleMappings {
        provider: "generic".to_owned(),
        group_claim: group_claim.to_owned(),
        default_role,
        roles: roles
            .into_iter()
            .map(|(role, groups)| (role, groups.into_iter().map(str::to_owned).collect()))
            .collect(),
    }
}

pub(crate) enum Stored {
    Absent,
    Text(&'static str),
    Unreadable,
}

pub(crate) struct FakeRows(pub(crate) Stored);

#[async_trait]
impl SsoSettingsRows for FakeRows {
    async fn stored_sso_settings(&self) -> Result<Option<String>, TableError> {
        match &self.0 {
            Stored::Absent => Ok(None),
            Stored::Text(text) => Ok(Some((*text).to_owned())),
            Stored::Unreadable => Err(TableError::MissingRow),
        }
    }
}
