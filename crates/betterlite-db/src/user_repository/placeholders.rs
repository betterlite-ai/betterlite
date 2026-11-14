use tokio_postgres::Row;

use crate::identity_rows::user::ScimPlaceholder;
use crate::postgres_store::diagnostics::DriverError;

pub(crate) const SHADOWING_PLACEHOLDERS_SQL: &str = r#"SELECT p.user_id AS placeholder_user_id,
       array_agg(r.user_id ORDER BY r.user_id) AS resolved_user_ids,
       p.teams AS team_ids
FROM "LiteLLM_UserTable" p
JOIN "LiteLLM_UserTable" r
  ON r.user_id <> p.user_id
 AND (r.sso_user_id = p.user_id OR LOWER(r.user_email) = LOWER(p.user_id))
WHERE p.sso_user_id IS NULL
  AND NOT EXISTS (SELECT 1 FROM "LiteLLM_VerificationToken" k WHERE k.user_id = p.user_id)
GROUP BY p.user_id, p.teams
ORDER BY p.user_id"#;

#[derive(Debug, thiserror::Error)]
pub enum PlaceholderError {
    #[error("the placeholder row column is unreadable")]
    Column {
        #[source]
        source: DriverError,
    },
    #[error("the placeholder row has no {field}")]
    MissingField { field: &'static str },
}

pub(crate) fn placeholder_row(row: &Row) -> Result<ScimPlaceholder, PlaceholderError> {
    let placeholder_user_id: String =
        row.try_get(0).map_err(|source| PlaceholderError::Column {
            source: DriverError { driver: source },
        })?;
    let resolved_user_ids: Option<Vec<String>> =
        row.try_get(1).map_err(|source| PlaceholderError::Column {
            source: DriverError { driver: source },
        })?;
    let team_ids: Option<Vec<String>> =
        row.try_get(2).map_err(|source| PlaceholderError::Column {
            source: DriverError { driver: source },
        })?;
    Ok(ScimPlaceholder {
        placeholder_user_id,
        resolved_user_ids: resolved_user_ids.ok_or(PlaceholderError::MissingField {
            field: "resolved_user_ids",
        })?,
        team_ids: team_ids.ok_or(PlaceholderError::MissingField { field: "team_ids" })?,
    })
}
