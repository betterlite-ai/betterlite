use std::sync::Arc;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use super::support::{CountedOrgRoles, caller, team_access};
use crate::model_write_fields::access::{ModelCall, ModelWriteAccess};
use crate::model_write_fields::error::ModelWriteError;
use crate::model_write_fields::teams::{TeamRows, WriterTeamRows};

const STORAGE: &str = r#"CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT NOT NULL,
    team_alias TEXT,
    organization_id TEXT,
    admins TEXT[] NOT NULL DEFAULT '{}',
    members TEXT[] NOT NULL DEFAULT '{}',
    models TEXT[] NOT NULL DEFAULT '{}',
    members_with_roles JSONB NOT NULL DEFAULT '{}',
    metadata JSONB NOT NULL DEFAULT '{}',
    model_spend JSONB NOT NULL DEFAULT '{}',
    model_max_budget JSONB NOT NULL DEFAULT '{}',
    router_settings JSONB DEFAULT '{}',
    budget_limits JSONB,
    max_budget DOUBLE PRECISION,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    CONSTRAINT "LiteLLM_TeamTable_pkey" PRIMARY KEY (team_id)
);
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, organization_id, members_with_roles, metadata) VALUES
    ('team-1', 'alpha', 'org-1', '[{"user_id":"admin-1","role":"admin"},{"user_id":"member-1","role":"user"}]'::JSONB, '{}'::JSONB);"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn writer_team_rows_read_stored_teams() {
    let url = Environment::parse_from(["model-write-fields-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![STORAGE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()
        .unwrap();
    let rows = WriterTeamRows {
        writer: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
    };
    let stored = rows.find_team("team-1").await.unwrap().unwrap();
    let roster: Vec<(Option<&str>, &MemberRole)> = stored
        .fields
        .base
        .members_with_roles
        .iter()
        .map(|member| (member.base.user_id.as_deref(), &member.role))
        .collect();
    let expected = vec![
        (Some("admin-1"), &MemberRole::Admin),
        (Some("member-1"), &MemberRole::User),
    ];
    assert_eq!(roster, expected);
    assert!(rows.find_team("gone").await.unwrap().is_none());
    let access = team_access(&Arc::new(CountedOrgRoles::default()));
    let writes = ModelWriteAccess {
        teams: &rows,
        team_access: &access,
    };
    writes
        .can_make_model_call(&caller("admin-1", None), Some("team-1"), ModelCall::Write)
        .await
        .unwrap();
    let refused = writes
        .can_make_model_call(&caller("member-1", None), Some("team-1"), ModelCall::Write)
        .await;
    assert!(matches!(refused, Err(ModelWriteError::NotTeamAdmin { .. })));
    pool.close();
    fixture.close_async().await.unwrap();
}
