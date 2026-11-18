use std::fmt::Display;
use std::future::Future;

use rust_decimal::Decimal;
use tokio_postgres::Row;

use crate::key_hash::KeyHash;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

// _BudgetLinkedRow: every linked table carries the tier link and the spend the cascade resets.
pub trait BudgetLinkedRow {
    fn spend(&self) -> Option<Decimal>;
    fn budget_id(&self) -> Option<&str>;
}

// Counter and cache-key formats for one linked table, so the reset job and the auth readers cannot drift apart.
pub trait LinkedKeys {
    fn counter_key(&self) -> String;
    fn cache_keys(&self) -> Vec<String>;
}

macro_rules! linked_row {
    ($name:ident, $($field:ident: $ty:ty),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            $(pub $field: $ty,)+
            pub budget_id: Option<String>,
            pub spend: Option<Decimal>,
        }

        impl BudgetLinkedRow for $name {
            fn spend(&self) -> Option<Decimal> {
                self.spend
            }

            fn budget_id(&self) -> Option<&str> {
                self.budget_id.as_deref()
            }
        }
    };
}

linked_row!(TeamMembershipRow, user_id: String, team_id: String);
linked_row!(KeyRow, token: KeyHash);
linked_row!(OrganizationRow, organization_id: String);
linked_row!(TagRow, tag_name: String);
linked_row!(ModelAccessGroupRow, access_group_name: String);
linked_row!(ProjectRow, project_id: String);
linked_row!(EndUserRow, user_id: String);

// The rows the cascade reads before the write, so their counters and cache entries can be dropped once the transaction commits.
pub trait LinkedRows {
    type Failure: Display + Send + 'static;

    fn team_memberships(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<TeamMembershipRow>, Self::Failure>> + Send;

    fn keys(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<KeyRow>, Self::Failure>> + Send;

    fn organizations(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<OrganizationRow>, Self::Failure>> + Send;

    fn tags(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<TagRow>, Self::Failure>> + Send;

    fn model_access_groups(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<ModelAccessGroupRow>, Self::Failure>> + Send;

    fn projects(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<ProjectRow>, Self::Failure>> + Send;
}

pub struct PostgresLinkedRows {
    pub store: PostgresStore,
}

// spend is read as numeric because the column is double precision and money stays a Decimal past the driver.
const TEAM_MEMBERSHIPS_SQL: &str = "SELECT user_id, team_id, budget_id, spend::numeric AS spend FROM \"LiteLLM_TeamMembership\" WHERE budget_id = ANY($1::text[])";
const KEYS_SQL: &str = "SELECT token, budget_id, spend::numeric AS spend FROM \"LiteLLM_VerificationToken\" WHERE budget_id = ANY($1::text[]) AND budget_duration IS NULL AND spend > 0";
const ORGANIZATIONS_SQL: &str = "SELECT organization_id, budget_id, spend::numeric AS spend FROM \"LiteLLM_OrganizationTable\" WHERE budget_id = ANY($1::text[]) AND spend > 0";
const TAGS_SQL: &str = "SELECT tag_name, budget_id, spend::numeric AS spend FROM \"LiteLLM_TagTable\" WHERE budget_id = ANY($1::text[]) AND spend > 0";
const MODEL_ACCESS_GROUPS_SQL: &str = "SELECT access_group_name, budget_id, spend::numeric AS spend FROM \"LiteLLM_ModelAccessGroupBudgetTable\" WHERE budget_id = ANY($1::text[]) AND spend > 0";
const PROJECTS_SQL: &str = "SELECT project_id, budget_id, spend::numeric AS spend FROM \"LiteLLM_ProjectTable\" WHERE budget_id = ANY($1::text[]) AND spend > 0";

async fn read_linked<Record, Decode>(
    store: &PostgresStore,
    sql: &str,
    budget_ids: &[String],
    decode: Decode,
) -> Result<Vec<Record>, DbError>
where
    Decode: FnMut(&Row) -> Result<Record, DbError>,
{
    let client = store.session().await?;
    let rows = client.query(sql, &[&budget_ids]).await.map_err(backend)?;
    rows.iter().map(decode).collect()
}

macro_rules! linked_reads {
    ($(($method:ident, $sql:ident, $row:ty, $decode:ident)),+ $(,)?) => {
        impl LinkedRows for PostgresLinkedRows {
            type Failure = DbError;

            $(
                async fn $method(&self, budget_ids: &[String]) -> Result<Vec<$row>, DbError> {
                    read_linked(&self.store, $sql, budget_ids, $decode).await
                }
            )+
        }
    };
}

linked_reads! {
    (
        team_memberships,
        TEAM_MEMBERSHIPS_SQL,
        TeamMembershipRow,
        decode_team_membership
    ),
    (keys, KEYS_SQL, KeyRow, decode_key),
    (organizations, ORGANIZATIONS_SQL, OrganizationRow, decode_organization),
    (tags, TAGS_SQL, TagRow, decode_tag),
    (
        model_access_groups,
        MODEL_ACCESS_GROUPS_SQL,
        ModelAccessGroupRow,
        decode_model_access_group
    ),
    (projects, PROJECTS_SQL, ProjectRow, decode_project),
}

fn decode_team_membership(row: &Row) -> Result<TeamMembershipRow, DbError> {
    Ok(TeamMembershipRow {
        user_id: row.try_get("user_id").map_err(backend)?,
        team_id: row.try_get("team_id").map_err(backend)?,
        budget_id: row.try_get("budget_id").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}

fn decode_key(row: &Row) -> Result<KeyRow, DbError> {
    Ok(KeyRow {
        token: KeyHash::from(row.try_get::<_, String>("token").map_err(backend)?),
        budget_id: row.try_get("budget_id").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}

fn decode_organization(row: &Row) -> Result<OrganizationRow, DbError> {
    Ok(OrganizationRow {
        organization_id: row.try_get("organization_id").map_err(backend)?,
        budget_id: row.try_get("budget_id").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}

fn decode_tag(row: &Row) -> Result<TagRow, DbError> {
    Ok(TagRow {
        tag_name: row.try_get("tag_name").map_err(backend)?,
        budget_id: row.try_get("budget_id").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}

fn decode_model_access_group(row: &Row) -> Result<ModelAccessGroupRow, DbError> {
    Ok(ModelAccessGroupRow {
        access_group_name: row.try_get("access_group_name").map_err(backend)?,
        budget_id: row.try_get("budget_id").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}

fn decode_project(row: &Row) -> Result<ProjectRow, DbError> {
    Ok(ProjectRow {
        project_id: row.try_get("project_id").map_err(backend)?,
        budget_id: row.try_get("budget_id").map_err(backend)?,
        spend: row.try_get("spend").map_err(backend)?,
    })
}
