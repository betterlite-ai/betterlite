use std::str::from_utf8;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_identity_history::SCHEMA_UPDATE;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::tokio_postgres::NoTls;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};

use super::{admin, anonymous, fixed_clock, principal};
use crate::api::stored_skill_reads::StoredSkillReads;
use crate::api::stored_skill_reads::cache::SkillCacheLimits;
use crate::api::stored_skill_reads::error::StoredSkillReadError;
use crate::api::stored_skill_reads::stored_skill::StoredSkill;
use crate::skills::store::SkillPage;
use crate::skills::store::repository::PrismaSkillRepository;

const ALICE_SKILL: &str = "litellm_skill_alice";
const BOB_SKILL: &str = "litellm_skill_bob";
const OLD_SKILL: &str = "litellm_skill_old";
const GHOST_SKILL: &str = "litellm_skill_ghost";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["stored-skill-reads"])
        .url
        .map(SecretString::from)
}

async fn provision() -> PostgresFixture {
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        test_url().as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    task.resolve().await.unwrap()
}

fn postgres_store(fixture: &PostgresFixture) -> PostgresStore {
    let config = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    PostgresStore { pool: Some(pool) }
}

async fn migrate(store: &PostgresStore) {
    let client = store.session().await.unwrap();
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .unwrap();
    client
        .batch_execute(from_utf8(SCHEMA_UPDATE.bytes).unwrap())
        .await
        .unwrap();
}

fn no_publish() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok::<(), TableError>(()) });
        future
    })
}

async fn insert_row(
    store: &PostgresStore,
    skill_id: &str,
    owner: &str,
    title: &str,
    created_at: &str,
    content: Option<&[u8]>,
) {
    store
        .session()
        .await
        .unwrap()
        .execute(
            "INSERT INTO \"LiteLLM_SkillsTable\" (skill_id, display_title, created_by, created_at, file_content) \
             VALUES ($1, $2, $3, $4::text::timestamp(3), $5)",
            &[&skill_id, &title, &owner, &created_at, &content],
        )
        .await
        .unwrap();
}

async fn set_title(store: &PostgresStore, skill_id: &str, title: &str) {
    store
        .session()
        .await
        .unwrap()
        .execute(
            "UPDATE \"LiteLLM_SkillsTable\" SET display_title = $2 WHERE skill_id = $1",
            &[&skill_id, &title],
        )
        .await
        .unwrap();
}

fn ids(skills: &[StoredSkill]) -> Vec<&str> {
    skills.iter().map(|skill| skill.skill_id.as_str()).collect()
}

fn service<'db>(client: PrismaClient<'db>) -> StoredSkillReads<PrismaSkillRepository<'db>> {
    StoredSkillReads::new(
        PrismaSkillRepository::new(Some(client), no_publish()),
        SkillCacheLimits::default(),
        fixed_clock(),
    )
    .unwrap()
}

#[tokio::test]
async fn stored_skill_reads_postgres() {
    let fixture = provision().await;
    let writer = postgres_store(&fixture);
    migrate(&writer).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    insert_row(
        &writer,
        ALICE_SKILL,
        "alice",
        "Alpha",
        "2026-01-01 00:00:00",
        Some(&[b'z'; 120][..]),
    )
    .await;
    insert_row(
        &writer,
        BOB_SKILL,
        "bob",
        "Beta",
        "2026-02-01 00:00:00",
        None,
    )
    .await;
    insert_row(
        &writer,
        OLD_SKILL,
        "alice",
        "Old",
        "2025-12-01 00:00:00",
        Some(b"zip".as_slice()),
    )
    .await;

    let reads = service(client);
    let public_page = reads.list_skills(SkillPage::default(), None).await.unwrap();
    assert_eq!(ids(&public_page), vec![BOB_SKILL, ALICE_SKILL, OLD_SKILL]);
    assert_eq!(public_page[0].file_content, None);
    assert_eq!(public_page[1].file_content, Some(vec![b'z'; 120]));
    assert_eq!(public_page[2].file_content, Some(b"zip".to_vec()));

    let alice = principal("alice");
    let alice_page = reads
        .list_skills(SkillPage::default(), Some(&alice))
        .await
        .unwrap();
    assert_eq!(ids(&alice_page), vec![ALICE_SKILL, OLD_SKILL]);
    let bob_page = reads
        .list_skills(SkillPage::default(), Some(&principal("bob")))
        .await
        .unwrap();
    assert_eq!(ids(&bob_page), vec![BOB_SKILL]);
    let admin_page = reads
        .list_skills(SkillPage::default(), Some(&admin("root")))
        .await
        .unwrap();
    assert_eq!(admin_page.len(), 3);
    let anonymous_page = reads
        .list_skills(SkillPage::default(), Some(&anonymous()))
        .await
        .unwrap();
    assert!(anonymous_page.is_empty());
    let second_page = reads
        .list_skills(
            SkillPage {
                limit: 1,
                offset: 1,
            },
            Some(&admin("root")),
        )
        .await
        .unwrap();
    assert_eq!(ids(&second_page), vec![ALICE_SKILL]);

    let skill = reads.get_skill(ALICE_SKILL, Some(&alice)).await.unwrap();
    assert_eq!(skill.display_title.as_deref(), Some("Alpha"));
    assert_eq!(skill.file_content, Some(vec![b'z'; 120]));
    assert!(matches!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("bob")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    assert!(matches!(
        reads
            .get_skill(GHOST_SKILL, Some(&alice))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));

    set_title(&writer, ALICE_SKILL, "Changed").await;
    assert!(matches!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("bob")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    let cached = reads.get_skill(ALICE_SKILL, Some(&alice)).await.unwrap();
    assert_eq!(cached.display_title.as_deref(), Some("Alpha"));
    let cached_reader = service(client);
    let refreshed = cached_reader
        .get_skill(ALICE_SKILL, Some(&alice))
        .await
        .unwrap();
    assert_eq!(refreshed.display_title.as_deref(), Some("Changed"));

    assert!(matches!(
        cached_reader
            .get_skill(GHOST_SKILL, Some(&alice))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    insert_row(
        &writer,
        GHOST_SKILL,
        "alice",
        "Returned",
        "2026-03-01 00:00:00",
        Some(b"back".as_slice()),
    )
    .await;
    assert!(matches!(
        cached_reader
            .get_skill(GHOST_SKILL, Some(&alice))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    let after_insert = service(client);
    let returned = after_insert
        .get_skill(GHOST_SKILL, Some(&alice))
        .await
        .unwrap();
    assert_eq!(returned.file_content, Some(b"back".to_vec()));

    let offline = StoredSkillReads::new(
        PrismaSkillRepository::new(None::<PrismaClient<'static>>, no_publish()),
        SkillCacheLimits::default(),
        fixed_clock(),
    )
    .unwrap();
    assert!(matches!(
        offline.get_skill(ALICE_SKILL, None).await.unwrap_err(),
        StoredSkillReadError::NoDatabase
    ));
    assert!(matches!(
        offline
            .list_skills(SkillPage::default(), None)
            .await
            .unwrap_err(),
        StoredSkillReadError::NoDatabase
    ));

    drop(reads);
    drop(cached_reader);
    drop(after_insert);
    drop(offline);
    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}
