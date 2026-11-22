#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_GuardrailsTable" (
    "guardrail_id" TEXT NOT NULL,
    "guardrail_name" TEXT NOT NULL,
    "litellm_params" JSONB NOT NULL,
    "guardrail_info" JSONB,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "reviewed_at" TIMESTAMP(3),
    "status" TEXT NOT NULL DEFAULT 'active',
    "submitted_at" TIMESTAMP(3),
    "team_id" TEXT,
    CONSTRAINT "LiteLLM_GuardrailsTable_pkey" PRIMARY KEY ("guardrail_id")
);
"#;

const TEAMLESS_SQL: &str = r#"INSERT INTO "LiteLLM_GuardrailsTable"
    (guardrail_id, guardrail_name, litellm_params, guardrail_info, status, submitted_at, updated_at)
    VALUES ($1, $2, $3::text::jsonb, $4::text::jsonb, $5, now(), now())"#;

const PARAMS_TEXT: &str = r#"{"guardrail":"generic_guardrail_api","mode":"pre_call","api_base":"https://guard.example.com/check"}"#;
const INFO_TEXT: &str = r#"{"team_guardrail":true}"#;

fn publisher(journal: &Arc<Mutex<Vec<String>>>) -> PublishChange<TableError> {
    let entries = Arc::clone(journal);
    Arc::new(move |object_type: &str| {
        entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(object_type.to_owned());
        let future: PublishFuture<TableError> = Box::pin(async { Ok(()) });
        future
    })
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["guardrail-submission-rows"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

async fn insert_teamless(pool: &Pool, id: &str) {
    let name = "teamless-guard";
    let status = "active";
    let client = pool.get().await.unwrap();
    client
        .execute(
            TEAMLESS_SQL,
            &[&id, &name, &PARAMS_TEXT, &INFO_TEXT, &status],
        )
        .await
        .unwrap();
}

fn later_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-02T03:05:05Z")
        .expect("the fixture instant")
        .with_timezone(&Utc)
}

fn new_submission(name: &str, team: &str, now: DateTime<Utc>) -> NewSubmission {
    NewSubmission {
        guardrail_name: name.to_owned(),
        litellm_params: PARAMS_TEXT.to_owned(),
        guardrail_info: INFO_TEXT.to_owned(),
        status: "pending_review".to_owned(),
        team_id: team.to_owned(),
        submitted_at: now,
        created_at: now,
        updated_at: now,
    }
}

fn submission_rows<'db>(
    store: &'db PostgresStore,
    availability: &'db Availability,
    journal: &Arc<Mutex<Vec<String>>>,
) -> PostgresSubmissionRows<'db> {
    PostgresSubmissionRows {
        client: Some(PrismaClient {
            writer: store,
            reader: store,
            availability,
        }),
        publish: publisher(journal),
    }
}

#[test]
fn storage_binds_the_config_sync_table() {
    assert_eq!(SubmissionRowsBinding::NAME, "litellm_guardrailstable");
    assert_eq!(SubmissionRowsBinding::TABLE, "LiteLLM_GuardrailsTable");
}

#[tokio::test]
async fn storage_stores_the_created_row_and_publishes() {
    let (fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = submission_rows(&store, &availability, &journal);
    let created = rows
        .create(new_submission("alpha", "team-a", fixed_now()))
        .await
        .expect("the insert succeeds");
    assert_eq!(
        journal.lock().unwrap().clone(),
        vec!["litellm_guardrailstable".to_owned()]
    );
    assert!(!created.guardrail_id.is_empty());
    assert_eq!(created.guardrail_name, "alpha");
    assert_eq!(created.status.as_deref(), Some("pending_review"));
    assert_eq!(created.team_id.as_deref(), Some("team-a"));
    assert_eq!(
        created.litellm_params,
        Some(ColumnJson::Text(PARAMS_TEXT.to_owned()))
    );
    assert_eq!(
        created.guardrail_info,
        Some(ColumnJson::Text(INFO_TEXT.to_owned()))
    );
    assert_eq!(created.submitted_at.map(Timestamp::utc), Some(fixed_now()));
    assert!(created.reviewed_at.is_none());
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn storage_reads_the_row_by_id_and_name() {
    let (fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = submission_rows(&store, &availability, &journal);
    let created = rows
        .create(new_submission("alpha", "team-a", fixed_now()))
        .await
        .expect("the insert succeeds");
    let found = rows
        .find_by_id(&created.guardrail_id)
        .await
        .expect("the read succeeds")
        .expect("the row exists");
    assert_eq!(found, created);
    assert!(
        rows.find_by_name("alpha")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    assert!(
        rows.find_by_name("absent")
            .await
            .expect("the read succeeds")
            .is_none()
    );
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn storage_skips_teamless_rows() {
    let (fixture, pool) = database().await;
    insert_teamless(&pool, "g-teamless").await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = submission_rows(&store, &availability, &journal);
    let all = rows
        .find_team_rows(TeamFilter::AnyTeam)
        .await
        .expect("the read succeeds");
    assert!(all.is_empty());
    assert!(
        rows.find_by_name("teamless-guard")
            .await
            .expect("the read succeeds")
            .is_some()
    );
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn storage_returns_team_rows_newest_first() {
    let (fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = submission_rows(&store, &availability, &journal);
    let older = rows
        .create(new_submission("alpha", "team-a", fixed_now()))
        .await
        .expect("the insert succeeds");
    let newer = rows
        .create(new_submission("beta", "team-b", later_now()))
        .await
        .expect("the insert succeeds");
    let all = rows
        .find_team_rows(TeamFilter::AnyTeam)
        .await
        .expect("the read succeeds");
    let ids: Vec<&str> = all.iter().map(|row| row.guardrail_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![newer.guardrail_id.as_str(), older.guardrail_id.as_str()]
    );
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn storage_filters_team_rows_to_the_listed_teams() {
    let (fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = submission_rows(&store, &availability, &journal);
    let mine = rows
        .create(new_submission("alpha", "team-a", fixed_now()))
        .await
        .expect("the insert succeeds");
    rows.create(new_submission("beta", "team-b", fixed_now()))
        .await
        .expect("the insert succeeds");
    let listed = rows
        .find_team_rows(TeamFilter::Teams(vec!["team-a".to_owned()]))
        .await
        .expect("the read succeeds");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].guardrail_id, mine.guardrail_id);
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn storage_returns_nothing_for_no_teams() {
    let (fixture, pool) = database().await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = submission_rows(&store, &availability, &journal);
    rows.create(new_submission("alpha", "team-a", fixed_now()))
        .await
        .expect("the insert succeeds");
    let none = rows
        .find_team_rows(TeamFilter::Teams(Vec::new()))
        .await
        .expect("the read succeeds");
    assert!(none.is_empty());
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}
