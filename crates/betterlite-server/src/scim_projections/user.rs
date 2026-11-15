use super::USER_SCHEMA;
use super::directory::{
    Directory, Metadata, is_attribute_list, is_object, parse_directory_metadata,
};
use super::error::ProjectionError;
use super::naming::scim_names;
use super::timestamps::resource_meta;
use super::truthiness::is_set;
use crate::scim_records::attribute::DirectoryAttribute;
use crate::scim_records::email::Email;
use crate::scim_records::lax_bool::ScimBool;
use crate::scim_records::user::{Active, EnterpriseUser, User, UserEmail, UserGroup, UserName};
use crate::scim_records::{
    ENTERPRISE_METADATA_KEY, ENTERPRISE_USER_SCHEMA, ENTITLEMENTS_METADATA_KEY, ROLES_METADATA_KEY,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use serde::de::DeserializeOwned;

const ACTIVE_METADATA_KEY: &str = "scim_active";

// the fields both LiteLLM_UserTable and NewUserResponse give the projection
pub struct ScimUserSource<'user> {
    pub user_id: &'user str,
    pub user_alias: Option<&'user str>,
    pub user_email: Option<&'user str>,
    pub metadata: Option<&'user Metadata>,
    pub teams: &'user [String],
    pub created_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
}

pub struct UserProjection {
    pub user: User,
    // the metadata keys whose malformed block was left out of the user
    pub dropped: Vec<&'static str>,
}

impl<'user> From<&'user LiteLLMUserTable> for ScimUserSource<'user> {
    fn from(user: &'user LiteLLMUserTable) -> Self {
        Self {
            user_id: &user.user_id,
            user_alias: user.user_alias.as_deref(),
            user_email: user.user_email.as_deref(),
            metadata: user.metadata.as_ref().map(|payload| &payload.0),
            teams: &user.teams,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

// an address without @ can be a generated id, so it is a userName but never an email
fn emails(user_email: Option<&str>) -> Result<Vec<UserEmail>, ProjectionError> {
    match user_email.filter(|address| address.contains('@')) {
        Some(address) => Ok(vec![UserEmail {
            value: Email::try_from(address.to_owned())
                .map_err(|source| ProjectionError::Email { source })?,
            kind: None,
            primary: Some(ScimBool(true)),
        }]),
        None => Ok(Vec::new()),
    }
}

// metadata is writable outside SCIM, so a bad block is logged and left out instead of failing the response
fn directory<T>(
    metadata: &Metadata,
    key: &'static str,
    well_formed: fn(&JsonData) -> bool,
    user_id: &str,
    dropped: &mut Vec<&'static str>,
) -> Result<Option<T>, ProjectionError>
where
    T: DeserializeOwned,
{
    match parse_directory_metadata(metadata, key, well_formed)
        .map_err(|source| ProjectionError::Encode { source })?
    {
        Directory::Absent => Ok(None),
        Directory::Parsed(block) => Ok(Some(block)),
        Directory::Malformed => {
            tracing::warn!("Skipping malformed {key} metadata on user {user_id} in SCIM response");
            dropped.push(key);
            Ok(None)
        }
    }
}

struct Blocks {
    enterprise: Option<EnterpriseUser>,
    entitlements: Option<Vec<DirectoryAttribute>>,
    roles: Option<Vec<DirectoryAttribute>>,
    dropped: Vec<&'static str>,
}

fn directory_blocks(metadata: &Metadata, id: &str) -> Result<Blocks, ProjectionError> {
    let mut dropped = Vec::new();
    let enterprise = directory(
        metadata,
        ENTERPRISE_METADATA_KEY,
        is_object,
        id,
        &mut dropped,
    )?;
    let entitlements = directory(
        metadata,
        ENTITLEMENTS_METADATA_KEY,
        is_attribute_list,
        id,
        &mut dropped,
    )?;
    let roles = directory(
        metadata,
        ROLES_METADATA_KEY,
        is_attribute_list,
        id,
        &mut dropped,
    )?;
    Ok(Blocks {
        enterprise,
        entitlements,
        roles,
        dropped,
    })
}

pub fn project_user(
    source: &ScimUserSource<'_>,
    groups: Vec<UserGroup>,
) -> Result<UserProjection, ProjectionError> {
    let no_metadata = Metadata::new();
    let metadata = source.metadata.unwrap_or(&no_metadata);
    let names = scim_names(source.user_email, source.user_alias, metadata)?;
    let blocks = directory_blocks(metadata, source.user_id)?;
    let mut schemas = vec![USER_SCHEMA.to_owned()];
    if blocks.enterprise.is_some() {
        schemas.push(ENTERPRISE_USER_SCHEMA.to_owned());
    }
    let user = User {
        schemas,
        id: Some(source.user_id.to_owned()),
        external_id: None,
        meta: Some(resource_meta("User", source.created_at, source.updated_at)?),
        user_name: Some(names.user_name.clone()),
        name: Some(UserName {
            family_name: Some(names.family_name),
            given_name: Some(names.given_name),
            formatted: None,
            middle_name: None,
            honorific_prefix: None,
            honorific_suffix: None,
        }),
        display_name: Some(names.user_name),
        active: Active::Sent(ScimBool(is_active(metadata))),
        emails: Some(emails(source.user_email)?),
        groups: Some(groups),
        entitlements: blocks.entitlements,
        roles: blocks.roles,
        enterprise: blocks.enterprise,
    };
    Ok(UserProjection {
        user,
        dropped: blocks.dropped,
    })
}

// a missing or null scim_active leaves the account active; any other stored value decides by itself
fn is_active(metadata: &Metadata) -> bool {
    match metadata.get(ACTIVE_METADATA_KEY) {
        None | Some(JsonData::Null) => true,
        Some(stored) => is_set(stored),
    }
}
