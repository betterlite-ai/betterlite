use serde::Serialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use thiserror::Error;

macro_rules! record {
    ($(#[$meta:meta])* $name:ident { $($fields:tt)* }) => {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        $(#[$meta])*
        pub struct $name { $($fields)* }
    };
}
macro_rules! schema_list {
    ($name:ident, $urn:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Vec<String>);
        impl Default for $name {
            fn default() -> Self {
                Self(vec![String::from($urn)])
            }
        }
    };
}
pub mod attribute;
pub mod discovery;
pub mod group;
mod tests;
pub mod user;
pub const ENTERPRISE_USER_SCHEMA: &str =
    "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User";
pub const ENTERPRISE_METADATA_KEY: &str = "scim_enterprise";
pub const ENTITLEMENTS_METADATA_KEY: &str = "scim_entitlements";
pub const ROLES_METADATA_KEY: &str = "scim_roles";
pub const MANAGED_TEAM_METADATA_KEY: &str = "scim_managed";
pub const TEAM_DATA_METADATA_KEY: &str = "scim_data";
// directory meta and extension members are stored and echoed as sent, never read by the proxy
pub type RawObject = BTreeMap<String, Box<RawValue>>;
#[derive(Debug, Error)]
pub enum ScimRecordError {
    #[error("the SCIM resource body is not valid: {0}")]
    Decode(#[source] serde_json::Error),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    User,
    Group,
    Patch,
}
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ScimResource {
    User(Box<user::User>),
    Group(group::Group),
    Patch(group::Patch),
}
pub fn decode_scim_resource(
    bytes: &[u8],
    kind: ResourceKind,
) -> Result<ScimResource, ScimRecordError> {
    let decoded = match kind {
        ResourceKind::User => serde_json::from_slice(bytes).map(ScimResource::User),
        ResourceKind::Group => serde_json::from_slice(bytes).map(ScimResource::Group),
        ResourceKind::Patch => serde_json::from_slice(bytes).map(ScimResource::Patch),
    };
    decoded.map_err(ScimRecordError::Decode)
}
