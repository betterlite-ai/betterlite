use crate::connection::ConnectionOptions;
use crate::error::FixtureError;
use crate::seed::{BudgetRow, FixtureSeed, JwtMapping, KeyRow, Membership, TeamRow, UserRow};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls};
const SCHEMA: &str = concat!(
    include_str!("../../fixtures/baseline.sql"),
    r#"
CREATE TABLE IF NOT EXISTS "LiteLLM_JWTKeyMapping" ("id" TEXT PRIMARY KEY,"jwt_claim_name" TEXT NOT NULL,"jwt_claim_value" TEXT NOT NULL,"token" TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS "LiteLLM_ManagedFileTable" ("id" TEXT PRIMARY KEY,"unified_file_id" TEXT NOT NULL,"file_object" JSONB NOT NULL,"model_mappings" JSONB NOT NULL);
CREATE TABLE IF NOT EXISTS "LiteLLM_ManagedObjectTable" ("id" TEXT PRIMARY KEY,"unified_object_id" TEXT NOT NULL,"file_object" JSONB NOT NULL,"model_mappings" JSONB NOT NULL);
CREATE TABLE IF NOT EXISTS "LiteLLM_ManagedVectorStoreTable" ("id" TEXT PRIMARY KEY,"unified_resource_id" TEXT NOT NULL,"model_mappings" JSONB NOT NULL);
"#
);
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScenarioSetup {
    pub users: Vec<UserRow>,
    pub teams: Vec<TeamRow>,
    pub memberships: Vec<Membership>,
    pub budgets: Vec<BudgetRow>,
    pub jwt_mappings: Vec<JwtMapping>,
    pub keys: Vec<KeyRow>,
    pub managed_files: Vec<ManagedFileRow>,
    pub managed_objects: Vec<ManagedObjectRow>,
    pub vector_stores: Vec<VectorStoreRow>,
    pub polling: Vec<PollingEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFileRow {
    pub unified_file_id: String,
    #[serde(default)]
    pub content: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedObjectRow {
    pub unified_object_id: String,
    #[serde(default)]
    pub content: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorStoreRow {
    pub unified_resource_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollingEntry {
    pub name: String,
    #[serde(default)]
    pub value: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupList {
    Users,
    Teams,
    Memberships,
    Budgets,
    JwtMappings,
    Keys,
    ManagedFiles,
    ManagedObjects,
    VectorStores,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SetupJournalEntry {
    pub list: SetupList,
    pub index: usize,
}
impl ScenarioSetup {
    pub fn row_count(&self) -> usize {
        let counts = [
            self.users.len(),
            self.teams.len(),
            self.memberships.len(),
            self.budgets.len(),
            self.jwt_mappings.len(),
            self.keys.len(),
            self.managed_files.len(),
            self.managed_objects.len(),
            self.vector_stores.len(),
        ];
        counts.into_iter().fold(0_usize, usize::saturating_add)
    }
    pub fn fixture_bytes(&self) -> usize {
        self.managed_files
            .iter()
            .map(|row| row.content.len())
            .chain(self.managed_objects.iter().map(|row| row.content.len()))
            .fold(0_usize, usize::saturating_add)
    }
}
pub fn apply_setup(
    options: &ConnectionOptions,
    setup: &ScenarioSetup,
) -> Result<Vec<SetupJournalEntry>, FixtureError> {
    let mut builder = Builder::new_current_thread();
    let runtime = builder
        .enable_all()
        .build()
        .map_err(FixtureError::Runtime)?;
    runtime.block_on(apply_async(options, setup))
}
async fn apply_async(
    options: &ConnectionOptions,
    setup: &ScenarioSetup,
) -> Result<Vec<SetupJournalEntry>, FixtureError> {
    let mut drivers = JoinSet::new();
    let (client, connection) = tokio_postgres::connect(options.url.expose_secret(), NoTls)
        .await
        .map_err(FixtureError::Connect)?;
    drivers.spawn(async move { connection.await.map_err(FixtureError::Driver) });
    client
        .batch_execute(SCHEMA)
        .await
        .map_err(FixtureError::Database)?;
    let journal = insert_setup(&client, setup).await;
    drop(client);
    let mut driver = Ok(());
    while let Some(result) = drivers.join_next().await {
        driver = result.map_err(FixtureError::DriverJoin).and(driver);
    }
    match journal {
        Ok(entries) => driver.map(|()| entries),
        Err(error) => Err(error),
    }
}
async fn insert_setup(
    client: &Client,
    setup: &ScenarioSetup,
) -> Result<Vec<SetupJournalEntry>, FixtureError> {
    let mut journal = Vec::new();
    insert_all(client, &setup.users, &mut journal).await?;
    insert_all(client, &setup.teams, &mut journal).await?;
    insert_all(client, &setup.memberships, &mut journal).await?;
    insert_all(client, &setup.budgets, &mut journal).await?;
    insert_jwt_mappings(client, setup, &mut journal).await?;
    insert_keys(client, setup, &mut journal).await?;
    insert_all(client, &setup.managed_files, &mut journal).await?;
    insert_all(client, &setup.managed_objects, &mut journal).await?;
    insert_all(client, &setup.vector_stores, &mut journal).await?;
    Ok(journal)
}
trait SeedRow {
    const LIST: SetupList;
    const SQL: &'static str;
    fn params(&self) -> Vec<&(dyn ToSql + Sync)>;
}
macro_rules! seed_row {
    ($row:ty, $list:expr, $sql:expr, $($field:ident),+) => {
        impl SeedRow for $row {
            const LIST: SetupList = $list;
            const SQL: &'static str = $sql;
            fn params(&self) -> Vec<&(dyn ToSql + Sync)> {
                vec![$(&self.$field),+]
            }
        }
    };
}
#[rustfmt::skip]
seed_row!(UserRow, SetupList::Users, "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\",\"user_email\") VALUES ($1,$2)", user_id, user_email);
#[rustfmt::skip]
seed_row!(TeamRow, SetupList::Teams, "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\",\"team_alias\") VALUES ($1,$2)", team_id, team_alias);
#[rustfmt::skip]
seed_row!(Membership, SetupList::Memberships, "INSERT INTO \"LiteLLM_TeamMembership\" (\"user_id\",\"team_id\") VALUES ($1,$2)", user_id, team_id);
#[rustfmt::skip]
seed_row!(BudgetRow, SetupList::Budgets, "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\",\"max_budget\",\"created_by\",\"updated_by\") VALUES ($1,$2::numeric::float8,$3,$4)", budget_id, max_budget, user_id, user_id);
#[rustfmt::skip]
seed_row!(ManagedFileRow, SetupList::ManagedFiles, "INSERT INTO \"LiteLLM_ManagedFileTable\" (\"id\",\"unified_file_id\",\"file_object\",\"model_mappings\") VALUES ($1,$2,jsonb_build_object('content',$3::text),'{}'::jsonb)", unified_file_id, unified_file_id, content);
#[rustfmt::skip]
seed_row!(ManagedObjectRow, SetupList::ManagedObjects, "INSERT INTO \"LiteLLM_ManagedObjectTable\" (\"id\",\"unified_object_id\",\"file_object\",\"model_mappings\") VALUES ($1,$2,jsonb_build_object('content',$3::text),'{}'::jsonb)", unified_object_id, unified_object_id, content);
#[rustfmt::skip]
seed_row!(VectorStoreRow, SetupList::VectorStores, "INSERT INTO \"LiteLLM_ManagedVectorStoreTable\" (\"id\",\"unified_resource_id\",\"model_mappings\") VALUES ($1,$2,'{}'::jsonb)", unified_resource_id, unified_resource_id);
async fn insert_all<T>(
    client: &Client,
    rows: &[T],
    journal: &mut Vec<SetupJournalEntry>,
) -> Result<(), FixtureError>
where
    T: SeedRow + Sync,
{
    let mut index = 0_usize;
    while let Some(row) = rows.get(index) {
        let params = row.params();
        client
            .execute(T::SQL, &params)
            .await
            .map_err(FixtureError::Database)?;
        let entry = SetupJournalEntry {
            list: T::LIST,
            index,
        };
        journal.push(entry);
        index = index.saturating_add(1);
    }
    Ok(())
}
async fn insert_jwt_mappings(
    client: &Client,
    setup: &ScenarioSetup,
    journal: &mut Vec<SetupJournalEntry>,
) -> Result<(), FixtureError> {
    let hashes = setup
        .jwt_mappings
        .iter()
        .map(|mapping| {
            setup
                .keys
                .iter()
                .find(|key| key.user_id == mapping.user_id)
                .map(|key| FixtureSeed::key_hash(key.token.expose_secret()))
                .ok_or_else(|| FixtureError::MissingReference {
                    relation: "JWT mapping",
                    kind: "key",
                    identity: mapping.user_id.clone(),
                })
        })
        .collect::<Result<Vec<String>, FixtureError>>()?;
    let sql = "INSERT INTO \"LiteLLM_JWTKeyMapping\" (\"id\",\"jwt_claim_name\",\"jwt_claim_value\",\"token\") VALUES ($1,'sub',$2,$3)";
    for (index, (mapping, hash)) in setup.jwt_mappings.iter().zip(hashes).enumerate() {
        let params: &[&(dyn ToSql + Sync)] = &[&mapping.subject, &mapping.subject, &hash];
        client
            .execute(sql, params)
            .await
            .map_err(FixtureError::Database)?;
        journal.push(SetupJournalEntry {
            list: SetupList::JwtMappings,
            index,
        });
    }
    Ok(())
}
async fn insert_keys(
    client: &Client,
    setup: &ScenarioSetup,
    journal: &mut Vec<SetupJournalEntry>,
) -> Result<(), FixtureError> {
    for (index, row) in setup.keys.iter().enumerate() {
        assign_user_role(client, row).await?;
        let hash = FixtureSeed::key_hash(row.token.expose_secret());
        insert_token(client, &hash, row).await?;
        journal.push(SetupJournalEntry {
            list: SetupList::Keys,
            index,
        });
    }
    Ok(())
}
async fn insert_token(client: &Client, hash: &str, row: &KeyRow) -> Result<(), FixtureError> {
    let params: &[&(dyn ToSql + Sync)] = &[&hash, &row.user_id, &row.team_id];
    let sql = "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\",\"user_id\",\"team_id\") VALUES ($1,$2,$3)";
    client
        .execute(sql, params)
        .await
        .map_err(FixtureError::Database)?;
    Ok(())
}
async fn assign_user_role(client: &Client, row: &KeyRow) -> Result<(), FixtureError> {
    let Some(role) = row.role else {
        return Ok(());
    };
    let sql = "UPDATE \"LiteLLM_UserTable\" SET \"user_role\" = $1 WHERE \"user_id\" = $2";
    client
        .execute(sql, &[&role.as_wire(), &row.user_id])
        .await
        .map_err(FixtureError::Database)?;
    Ok(())
}
