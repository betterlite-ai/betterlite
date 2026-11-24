use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_org_lookup::error::OrgLookupError;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::project::get_project_object;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_user_lookup::store::{UserStore, UserStoreError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::key_repository::decode::{TokenDecodeError, decode_mapping};
use betterlite_db::key_repository::table::columns::TokenColumn;
use betterlite_db::key_repository::table::{TokenFilter, TokenTableError, TokenValue};
use betterlite_db::key_repository::{TokenRepositoryError, VerificationTokenRepository};
use betterlite_db::postgres_store::session::PostgresStore;
use serde::Deserialize;
use serde_json::Error as JsonError;

use crate::key_list_filters::postgres::PostgresTokenClient;

// the two find_many(where=...) selectors LiteLLM builds from a team or an organization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOwner<'owner> {
    Team(&'owner str),
    Organization(&'owner str),
}

impl<'owner> KeyOwner<'owner> {
    #[must_use]
    pub const fn column(self) -> TokenColumn {
        match self {
            Self::Team(_) => TokenColumn::TeamId,
            Self::Organization(_) => TokenColumn::OrganizationId,
        }
    }

    #[must_use]
    pub const fn id(self) -> &'owner str {
        match self {
            Self::Team(team_id) => team_id,
            Self::Organization(organization_id) => organization_id,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyLimitReadError {
    #[error("the key table is unavailable")]
    Table(#[source] TokenRepositoryError<Infallible>),
    #[error("the key query failed")]
    Keys(#[source] TokenTableError),
    #[error("a stored key row is invalid")]
    Decode(#[source] TokenDecodeError),
    #[error("the project lookup failed")]
    Project(#[source] OrgLookupError),
    #[error("the user lookup failed")]
    User(#[source] UserStoreError),
    #[error("the user's organization memberships are invalid")]
    Membership(#[source] JsonError),
}

#[async_trait]
pub trait KeyLimitReads: Send + Sync {
    async fn keys(
        &self,
        owner: KeyOwner<'_>,
    ) -> Result<Vec<LiteLLMVerificationToken>, KeyLimitReadError>;

    async fn project(&self, project_id: &str) -> Result<Option<CachedProject>, KeyLimitReadError>;

    async fn member_organizations(
        &self,
        user_id: &str,
    ) -> Result<BTreeSet<String>, KeyLimitReadError>;
}

pub struct PostgresKeyLimitReads {
    pub store: PostgresStore,
    pub projects: OrgLookup,
    pub users: Arc<dyn UserStore>,
    pub now: f64,
}

#[async_trait]
impl KeyLimitReads for PostgresKeyLimitReads {
    async fn keys(
        &self,
        owner: KeyOwner<'_>,
    ) -> Result<Vec<LiteLLMVerificationToken>, KeyLimitReadError> {
        let client = PostgresTokenClient { store: &self.store };
        let repository = VerificationTokenRepository {
            database: Some(WriterPinnedClient::Writer(&client)),
        };
        let table = repository.table().map_err(KeyLimitReadError::Table)?;
        let filter = TokenFilter(vec![(
            owner.column(),
            TokenValue::Text(owner.id().to_owned()),
        )]);
        let rows = table
            .find_many(&filter, None)
            .await
            .map_err(KeyLimitReadError::Keys)?;
        rows.into_iter().map(|row| decode_row(row.0)).collect()
    }

    async fn project(&self, project_id: &str) -> Result<Option<CachedProject>, KeyLimitReadError> {
        get_project_object(&self.projects, project_id, self.now)
            .await
            .map_err(KeyLimitReadError::Project)
    }

    async fn member_organizations(
        &self,
        user_id: &str,
    ) -> Result<BTreeSet<String>, KeyLimitReadError> {
        let row = self
            .users
            .find_by_id(user_id, false)
            .await
            .map_err(KeyLimitReadError::User)?;
        row.map_or_else(|| Ok(BTreeSet::new()), |found| member_ids(&found))
    }
}

fn decode_row(row: JsonData) -> Result<LiteLLMVerificationToken, KeyLimitReadError> {
    let JsonData::Object(fields) = row else {
        return Err(KeyLimitReadError::Decode(TokenDecodeError::NotAnObject));
    };
    decode_mapping(fields).map_err(KeyLimitReadError::Decode)
}

#[derive(Debug, Deserialize)]
struct MembershipRow {
    #[serde(default)]
    organization_id: Option<String>,
}

// membership.organization_id for each row in (memberships or []), skipping the rows without one
pub fn member_ids(row: &JsonData) -> Result<BTreeSet<String>, KeyLimitReadError> {
    let JsonData::Object(fields) = row else {
        return Ok(BTreeSet::new());
    };
    let Some(JsonData::Array(entries)) = fields.get("organization_memberships") else {
        return Ok(BTreeSet::new());
    };
    let mut found = BTreeSet::new();
    for entry in entries {
        let wire = serde_json::to_value(entry).map_err(KeyLimitReadError::Membership)?;
        let membership: MembershipRow =
            serde_json::from_value(wire).map_err(KeyLimitReadError::Membership)?;
        if let Some(id) = membership.organization_id {
            found.insert(id);
        }
    }
    Ok(found)
}
