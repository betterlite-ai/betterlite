use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::{DbSession, PostgresStore};
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};

use crate::budget_email::BudgetEmailError;

const TEAM_MEMBER_EMAIL_SQL: &str =
    r#"SELECT user_email FROM "LiteLLM_UserTable" WHERE user_id = ANY($1::TEXT[])"#;

pub async fn get_all_team_member_emails(
    database: Option<&PostgresStore>,
    team_id: Option<&str>,
) -> Result<Vec<String>, BudgetEmailError> {
    tracing::debug!(
        "Email Alerting: Getting all team members for team_id={}",
        team_id.unwrap_or_default()
    );
    let Some(found_team_id) = team_id else {
        return Ok(Vec::new());
    };
    let Some(store) = database else {
        return Err(BudgetEmailError::NoDatabase);
    };
    let Some(user_ids) = team_member_ids(store, found_team_id).await? else {
        return Ok(Vec::new());
    };
    tracing::debug!(
        "Email Alerting: Got {} team members for team_id={found_team_id}",
        user_ids.len()
    );
    emails_for_user_ids(store, &user_ids).await
}

async fn team_member_ids(
    store: &PostgresStore,
    team_id: &str,
) -> Result<Option<Vec<String>>, BudgetEmailError> {
    let repository = TeamRepository {
        database: TeamDatabase {
            db: store,
            writer_db: store,
        },
        use_writer: false,
    };
    let Some(team) = repository.find_by_id(team_id).await? else {
        return Ok(None);
    };
    let mut user_ids = Vec::new();
    for member in &team.fields.base.members_with_roles {
        if let Some(member_user_id) = member
            .base
            .user_id
            .as_deref()
            .filter(|user_id| !user_id.is_empty())
        {
            user_ids.push(member_user_id.to_owned());
        }
    }
    Ok(Some(user_ids))
}

async fn emails_for_user_ids(
    store: &PostgresStore,
    user_ids: &[String],
) -> Result<Vec<String>, BudgetEmailError> {
    let mut client = store.session().await?;
    let session = client.transaction().await.map_err(backend)?;
    let emails = member_emails(&session, user_ids).await?;
    session.commit().await.map_err(backend)?;
    Ok(emails)
}

async fn member_emails(
    session: &DbSession<'_>,
    user_ids: &[String],
) -> Result<Vec<String>, DbError> {
    let rows = session
        .query(TEAM_MEMBER_EMAIL_SQL, &[&user_ids])
        .await
        .map_err(backend)?;
    let mut emails = Vec::with_capacity(rows.len());
    for row in &rows {
        if let Some(stored_email) = row
            .try_get::<_, Option<String>>("user_email")
            .map_err(backend)?
        {
            emails.push(stored_email);
        }
    }
    Ok(emails)
}
