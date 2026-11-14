use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};

use super::error::ScimMemberIdentityError;
use super::member_value::{is_strip_whitespace, metadata_has_scim_provenance};

// Prisma's find_many gives up after two rows, and LIMIT NULL is Postgres for no limit
const NAMED_USERS_SQL: &str = r#"SELECT "user_id" FROM "LiteLLM_UserTable" WHERE "sso_user_id" = $1 OR LOWER("user_email") = LOWER($2) ORDER BY "user_id" LIMIT $3"#;
const ACCOUNTS_NAMED_SQL: &str = r#"SELECT "user_id" FROM "LiteLLM_UserTable" WHERE "user_id" = $1 OR "sso_user_id" = $2 OR LOWER("user_email") = LOWER($2) ORDER BY "user_id" LIMIT 2"#;
const KEYED_USER_SQL: &str = r#"SELECT 1 FROM "LiteLLM_UserTable" WHERE "user_id" = $1"#;

pub struct MemberResolver<'database> {
    pub store: &'database PostgresStore,
}

impl<'database> MemberResolver<'database> {
    // an email matches the way new_user matches one, case-insensitively, while an SSO subject is exact
    pub async fn users_named_by_member_value(
        &self,
        value: &str,
        take: Option<i64>,
    ) -> Result<Vec<String>, ScimMemberIdentityError> {
        let subject = value.trim_matches(is_strip_whitespace);
        let client = self.store.session().await?;
        let rows = client
            .query(NAMED_USERS_SQL, &[&subject, &subject, &take])
            .await
            .map_err(|driver| ScimMemberIdentityError::Database {
                source: backend(driver),
            })?;
        rows.iter()
            .map(|row| row.try_get::<_, String>(0))
            .collect::<Result<Vec<String>, _>>()
            .map_err(|driver| ScimMemberIdentityError::Database {
                source: backend(driver),
            })
    }

    // the id compares exactly and unstripped, and a bounded scan that could have omitted it is completed below
    pub async fn accounts_named_by_member_value(
        &self,
        value: &str,
    ) -> Result<Vec<String>, ScimMemberIdentityError> {
        let subject = value.trim_matches(is_strip_whitespace);
        let client = self.store.session().await?;
        let rows = client
            .query(ACCOUNTS_NAMED_SQL, &[&value, &subject, &subject])
            .await
            .map_err(|driver| ScimMemberIdentityError::Database {
                source: backend(driver),
            })?;
        let mut named: Vec<String> = rows
            .iter()
            .map(|row| row.try_get::<_, String>(0))
            .collect::<Result<_, _>>()
            .map_err(|driver| ScimMemberIdentityError::Database {
                source: backend(driver),
            })?;
        if named.len() < 2 || named.iter().any(|found| found == value) {
            return Ok(named);
        }
        let keyed = client
            .query_opt(KEYED_USER_SQL, &[&value])
            .await
            .map_err(|driver| ScimMemberIdentityError::Database {
                source: backend(driver),
            })?;
        if keyed.is_some() {
            named.insert(0, value.to_owned());
        }
        Ok(named)
    }

    pub async fn is_scim_managed_team(
        &self,
        team_id: &str,
    ) -> Result<bool, ScimMemberIdentityError> {
        let teams = TeamRepository {
            database: TeamDatabase {
                db: self.store,
                writer_db: self.store,
            },
            use_writer: false,
        };
        let Some(team) = teams.find_by_id(team_id).await? else {
            return Ok(false);
        };
        Ok(team
            .fields
            .base
            .metadata
            .as_ref()
            .is_some_and(metadata_has_scim_provenance))
    }
}
