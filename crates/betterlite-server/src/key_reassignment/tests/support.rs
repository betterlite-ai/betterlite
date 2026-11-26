use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_model_patterns::view::DeploymentView;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::team_access::access::{OrgRoles, TeamAccess};
use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;

// the columns the three owned reads touch; every other row field decodes from its serde default
pub(super) const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT,
    models TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    user_id TEXT,
    team_id TEXT,
    tpm_limit BIGINT,
    rpm_limit BIGINT,
    object_permission_id TEXT
);
CREATE TABLE "LiteLLM_ObjectPermissionTable" (
    object_permission_id TEXT PRIMARY KEY,
    models TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[]
);
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT PRIMARY KEY,
    max_budget DOUBLE PRECISION
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Database {
    fixture: PostgresFixture,
    pool: Pool,
    pub(super) store: PostgresStore,
    availability: Availability,
}

impl Database {
    pub(super) async fn new(seed: &str) -> Self {
        let url = Environment::parse_from(["key-reassignment-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .unwrap();
        let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
        let pool = Pool::builder(manager).max_size(2).build().unwrap();
        pool.get().await.unwrap().batch_execute(seed).await.unwrap();
        let store = PostgresStore {
            pool: Some(pool.clone()),
        };
        Self {
            fixture,
            pool,
            store,
            availability: Availability::default(),
        }
    }

    pub(super) fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    pub(super) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

pub(super) fn team_row(text: &str) -> LiteLLMTeamTable {
    LiteLLMTeamTable::try_from(json(text)).unwrap()
}

pub(super) fn key_row(text: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(text).unwrap()
}

pub(super) fn caller(text: &str) -> TeamCaller {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

struct NoOrgAdmins;

#[async_trait]
impl OrgRoles for NoOrgAdmins {
    async fn is_org_admin(
        &self,
        _user_id: &str,
        _organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        Ok(false)
    }
}

// the B800 policy and B886 model grant a team move reads, with no aliases and no deployments
pub(super) struct Policy {
    pub(super) access: TeamAccess,
    aliases: BTreeMap<String, String>,
    view: DeploymentView,
    provider: UnresolvedProviderInference,
}

pub(super) fn caller_policy() -> Policy {
    Policy {
        access: TeamAccess {
            org_roles: Arc::new(NoOrgAdmins),
        },
        aliases: BTreeMap::new(),
        view: DeploymentView::default(),
        provider: UnresolvedProviderInference,
    }
}

impl Policy {
    pub(super) fn context(&self) -> ModelAccessContext<'_> {
        ModelAccessContext {
            view: Some(&self.view),
            global_model_aliases: &self.aliases,
            compaction: None,
            provider: &self.provider,
            max_recurse_depth: 100,
        }
    }
}
