#![cfg(test)]

use betterlite_postgres_fixture::connection::{ConnectionOptions, PostgresFixture};
use betterlite_postgres_fixture::fixtures::http_scenarios::{
    ManagedFileRow, ManagedObjectRow, ScenarioSetup, SetupJournalEntry, SetupList, VectorStoreRow,
    apply_setup,
};
use betterlite_postgres_fixture::seed::{
    BudgetRow, CallerRole, JwtMapping, KeyRow, Membership, TeamRow, UserRow,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tokio_postgres::NoTls;

use crate::runner::DatabaseEnvironment;

const SEEDED_TABLES: [&str; 9] = [
    "LiteLLM_UserTable",
    "LiteLLM_TeamTable",
    "LiteLLM_TeamMembership",
    "LiteLLM_BudgetTable",
    "LiteLLM_JWTKeyMapping",
    "LiteLLM_VerificationToken",
    "LiteLLM_ManagedFileTable",
    "LiteLLM_ManagedObjectTable",
    "LiteLLM_ManagedVectorStoreTable",
];

#[test]
fn postgres_setup_writes_every_list_in_the_task_order() {
    let Some(admin_url) = postgres_admin_url() else {
        return;
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let entered = runtime.enter();
    let mut task = PostgresFixture::provision_schema(Some(&admin_url));
    let fixture = runtime
        .block_on(task.resolve())
        .expect("the fixture schema is provisioned");
    drop(entered);
    let journal =
        apply_setup(&fixture.options, &seeded_setup()).expect("the setup rows are inserted");
    assert_eq!(journal, expected_seed_journal());
    let (counts, role) = runtime
        .block_on(seeded_state(&fixture.options))
        .expect("the seeded rows and the key's role are read back");
    assert_eq!(counts, vec![1_i64; 9]);
    assert_eq!(role, "internal_user");
    fixture.close().expect("the fixture schema is dropped");
}

fn postgres_admin_url() -> Option<SecretString> {
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")]).ok()?;
    environment.postgres_test_url
}

fn seeded_setup() -> ScenarioSetup {
    let (managed_files, managed_objects, vector_stores) = seeded_managed();
    ScenarioSetup {
        users: vec![UserRow {
            user_id: "user-a".to_owned(),
            user_email: Some("user-a@example.com".to_owned()),
        }],
        teams: vec![TeamRow {
            team_id: "team-a".to_owned(),
            team_alias: None,
        }],
        memberships: vec![Membership {
            user_id: "user-a".to_owned(),
            team_id: "team-a".to_owned(),
        }],
        budgets: vec![BudgetRow {
            budget_id: "budget-a".to_owned(),
            user_id: "user-a".to_owned(),
            max_budget: None,
        }],
        jwt_mappings: vec![JwtMapping {
            subject: "subject-a".to_owned(),
            user_id: "user-a".to_owned(),
        }],
        keys: vec![KeyRow {
            token: SecretString::from("sk-scenario-seed"),
            user_id: "user-a".to_owned(),
            team_id: None,
            role: Some(CallerRole::InternalUser),
        }],
        managed_files,
        managed_objects,
        vector_stores,
        polling: Vec::new(),
    }
}

fn seeded_managed() -> (
    Vec<ManagedFileRow>,
    Vec<ManagedObjectRow>,
    Vec<VectorStoreRow>,
) {
    (
        vec![ManagedFileRow {
            unified_file_id: "file-a".to_owned(),
            content: "file-body".to_owned(),
        }],
        vec![ManagedObjectRow {
            unified_object_id: "object-a".to_owned(),
            content: "object-body".to_owned(),
        }],
        vec![VectorStoreRow {
            unified_resource_id: "store-a".to_owned(),
        }],
    )
}

fn expected_seed_journal() -> Vec<SetupJournalEntry> {
    [
        SetupList::Users,
        SetupList::Teams,
        SetupList::Memberships,
        SetupList::Budgets,
        SetupList::JwtMappings,
        SetupList::Keys,
        SetupList::ManagedFiles,
        SetupList::ManagedObjects,
        SetupList::VectorStores,
    ]
    .into_iter()
    .map(|list| SetupJournalEntry { list, index: 0 })
    .collect()
}

async fn seeded_state(
    options: &ConnectionOptions,
) -> Result<(Vec<i64>, String), tokio_postgres::Error> {
    let mut drivers = JoinSet::new();
    let (client, connection) = tokio_postgres::connect(options.url.expose_secret(), NoTls).await?;
    drivers.spawn(connection);
    let mut counts = Vec::new();
    for table in SEEDED_TABLES {
        let row = client
            .query_one(&format!("SELECT count(*) FROM \"{table}\""), &[])
            .await?;
        counts.push(row.get::<_, i64>(0));
    }
    let role = client
        .query_one(
            "SELECT \"user_role\" FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = 'user-a'",
            &[],
        )
        .await?;
    drop(client);
    while let Some(result) = drivers.join_next().await {
        result.expect("the driver task joins")?;
    }
    Ok((counts, role.get(0)))
}
