use betterlite_config::scim_role_settings::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamQueryError, TeamRepository};
use betterlite_db::user_repository::{RowDecodeError, UserRepository};
use thiserror::Error;

use crate::scim_records::user::{GroupKind, UserGroup};

use super::roles::{default_scim_user_role, resolve_scim_user_role};

const UPDATE_ROLE_SQL: &str = r#"UPDATE "LiteLLM_UserTable" SET user_role = $2 WHERE user_id = $1"#;

#[derive(Debug, Error)]
pub enum ScimRoleError {
    #[error("the SCIM role recompute query failed")]
    Database {
        #[from]
        source: DbError,
    },
    #[error("a stored SCIM user row cannot be decoded")]
    User {
        #[from]
        source: DbError<RowDecodeError>,
    },
    #[error("a stored SCIM team row cannot be decoded")]
    Team {
        #[from]
        source: TeamQueryError,
    },
}

async fn find_user(
    store: &PostgresStore,
    user_id: &str,
) -> Result<Option<LiteLLMUserTable>, ScimRoleError> {
    let client = store.session().await?;
    let users = UserRepository {
        db: Some(&client),
        writer_db: None,
        use_writer: false,
    };
    Ok(users.find_by_id(user_id).await?)
}

async fn update_user_role(store: &PostgresStore, user_id: &str, role: &str) -> Result<(), DbError> {
    let client = store.session().await?;
    client
        .execute(UPDATE_ROLE_SQL, &[&user_id, &role])
        .await
        .map_err(backend)?;
    Ok(())
}

// a team that no longer exists still contributes its id as the group value
fn team_alias(team: &TeamRecord) -> Option<String> {
    let Some(JsonData::String(alias)) = team.fields.get("team_alias") else {
        return None;
    };
    Some(alias.clone())
}

pub async fn scim_groups_from_team_ids(
    store: &PostgresStore,
    team_ids: &[String],
) -> Result<Vec<UserGroup>, ScimRoleError> {
    let teams = TeamRepository {
        database: TeamDatabase {
            db: store,
            writer_db: store,
        },
        use_writer: false,
    };
    let mut groups = Vec::with_capacity(team_ids.len());
    for team_id in team_ids {
        let found = teams.table().find_unique(team_id).await?;
        let display = found.as_ref().and_then(team_alias);
        groups.push(UserGroup {
            value: team_id.clone(),
            display,
            kind: GroupKind::default(),
        });
    }
    Ok(groups)
}

pub async fn recompute_scim_member_roles(
    store: &PostgresStore,
    user_ids: &[String],
    settings: &Settings,
) -> Result<(), ScimRoleError> {
    let Some(admin_group) = settings.scim_admin_group.as_deref() else {
        return Ok(());
    };
    let default_role = default_scim_user_role(settings);
    for user_id in user_ids {
        let Some(user) = find_user(store, user_id).await? else {
            continue;
        };
        let groups = scim_groups_from_team_ids(store, &user.teams).await?;
        let Some(resolved) = resolve_scim_user_role(&groups, Some(admin_group), default_role)
        else {
            continue;
        };
        let role_text = resolved.to_string();
        update_user_role(store, user_id, &role_text).await?;
    }
    Ok(())
}
