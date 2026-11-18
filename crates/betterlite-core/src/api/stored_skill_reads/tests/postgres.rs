use std::str::from_utf8;
use std::sync::Arc;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_identity_history::SCHEMA_UPDATE;
use betterlite_postgres_fixture::connection::PostgresFixture;
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
    let table = from_utf8(SCHEMA_UPDATE.bytes).unwrap().to_owned();
    PostgresFixture::provision_with_statements(test_url().as_ref(), vec![table])
        .resolve()
        .await
        .unwrap()
}

fn postgres_store(fixture: &PostgresFixture) -> PostgresStore {
    let config = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    PostgresStore { pool: Some(pool) }
}

fn no_publish() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok::<(), TableError>(()) });
        future
    })
}

struct NewRow<'text> {
    skill_id: &'text str,
    owner: &'text str,
    title: &'text str,
    created_at: &'text str,
    content: Option<&'text [u8]>,
}

async fn insert_row(store: &PostgresStore, row: &NewRow<'_>) {
    store
        .session()
        .await
        .unwrap()
        .execute(
            "INSERT INTO \"LiteLLM_SkillsTable\" (skill_id, display_title, created_by, created_at, file_content) \
             VALUES ($1, $2, $3, $4::text::timestamp(3), $5)",
            &[
                &row.skill_id,
                &row.title,
                &row.owner,
                &row.created_at,
                &row.content,
            ],
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

fn service(client: PrismaClient<'_>) -> StoredSkillReads<PrismaSkillRepository<'_>> {
    StoredSkillReads::new(
        PrismaSkillRepository::new(Some(client), no_publish()),
        SkillCacheLimits::default(),
        fixed_clock(),
    )
    .unwrap()
}

async fn seeded_writer(fixture: &PostgresFixture) -> PostgresStore {
    let writer = postgres_store(fixture);
    insert_row(
        &writer,
        &NewRow {
            skill_id: ALICE_SKILL,
            owner: "alice",
            title: "Alpha",
            created_at: "2026-01-01 00:00:00",
            content: Some(&[b'z'; 120][..]),
        },
    )
    .await;
    insert_row(
        &writer,
        &NewRow {
            skill_id: BOB_SKILL,
            owner: "bob",
            title: "Beta",
            created_at: "2026-02-01 00:00:00",
            content: None,
        },
    )
    .await;
    insert_row(
        &writer,
        &NewRow {
            skill_id: OLD_SKILL,
            owner: "alice",
            title: "Old",
            created_at: "2025-12-01 00:00:00",
            content: Some(b"zip".as_slice()),
        },
    )
    .await;
    writer
}

async fn assert_public_page(reads: &StoredSkillReads<PrismaSkillRepository<'_>>) {
    let page = reads.list_skills(SkillPage::default(), None).await.unwrap();
    assert_eq!(ids(&page), vec![BOB_SKILL, ALICE_SKILL, OLD_SKILL]);
    assert_eq!(page[0].file_content, None);
    assert_eq!(page[1].file_content, Some(vec![b'z'; 120]));
    assert_eq!(page[2].file_content, Some(b"zip".to_vec()));
}

async fn assert_page(
    reads: &StoredSkillReads<PrismaSkillRepository<'_>>,
    principal: Option<&Principal>,
    page: SkillPage,
    expected: &[&str],
) {
    let listed = reads.list_skills(page, principal).await.unwrap();
    assert_eq!(ids(&listed), expected);
}

async fn assert_listing_pages(reads: &StoredSkillReads<PrismaSkillRepository<'_>>) {
    assert_public_page(reads).await;
    assert_page(
        reads,
        None,
        SkillPage::default(),
        &[BOB_SKILL, ALICE_SKILL, OLD_SKILL],
    )
    .await;
    let alice = principal("alice");
    assert_page(
        reads,
        Some(&alice),
        SkillPage::default(),
        &[ALICE_SKILL, OLD_SKILL],
    )
    .await;
    assert_page(
        reads,
        Some(&principal("bob")),
        SkillPage::default(),
        &[BOB_SKILL],
    )
    .await;
    assert_page(
        reads,
        Some(&admin("root")),
        SkillPage::default(),
        &[BOB_SKILL, ALICE_SKILL, OLD_SKILL],
    )
    .await;
    assert_page(reads, Some(&anonymous()), SkillPage::default(), &[]).await;
    let second = SkillPage {
        limit: 1,
        offset: 1,
    };
    assert_page(reads, Some(&admin("root")), second, &[ALICE_SKILL]).await;
}

async fn assert_owner_gets(reads: &StoredSkillReads<PrismaSkillRepository<'_>>) {
    let alice = principal("alice");
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
}

async fn assert_cached_row_survives_a_title_change(
    reads: &StoredSkillReads<PrismaSkillRepository<'_>>,
    client: PrismaClient<'_>,
    writer: &PostgresStore,
) {
    let alice = principal("alice");
    set_title(writer, ALICE_SKILL, "Changed").await;
    assert!(matches!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("bob")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    let cached = reads.get_skill(ALICE_SKILL, Some(&alice)).await.unwrap();
    assert_eq!(cached.display_title.as_deref(), Some("Alpha"));
    let refreshed_reads = service(client);
    let refreshed = refreshed_reads
        .get_skill(ALICE_SKILL, Some(&alice))
        .await
        .unwrap();
    assert_eq!(refreshed.display_title.as_deref(), Some("Changed"));
    assert_negative_cache_hides_a_new_row(&refreshed_reads, client, writer).await;
}

async fn assert_negative_cache_hides_a_new_row(
    reads: &StoredSkillReads<PrismaSkillRepository<'_>>,
    client: PrismaClient<'_>,
    writer: &PostgresStore,
) {
    let alice = principal("alice");
    assert!(matches!(
        reads
            .get_skill(GHOST_SKILL, Some(&alice))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    insert_row(
        writer,
        &NewRow {
            skill_id: GHOST_SKILL,
            owner: "alice",
            title: "Returned",
            created_at: "2026-03-01 00:00:00",
            content: Some(b"back".as_slice()),
        },
    )
    .await;
    assert!(matches!(
        reads
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
}

async fn assert_offline_reads_report_no_database() {
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
}

#[tokio::test]
async fn stored_skill_reads_postgres() {
    let fixture = provision().await;
    let writer = seeded_writer(&fixture).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let reads = service(client);
    assert_listing_pages(&reads).await;
    assert_owner_gets(&reads).await;
    assert_cached_row_survives_a_title_change(&reads, client, &writer).await;
    assert_offline_reads_report_no_database().await;
    drop(reads);
    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}
