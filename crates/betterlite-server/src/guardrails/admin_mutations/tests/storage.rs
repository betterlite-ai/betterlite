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

const INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_GuardrailsTable"
    (guardrail_id, guardrail_name, litellm_params, guardrail_info, status, submitted_at, updated_at)
    VALUES ($1, $2, $3::text::jsonb, $4::text::jsonb, $5, now(), now())"#;

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
    let url = Environment::parse_from(["guardrail-review-rows"])
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

async fn insert_row(pool: &Pool, id: &str, params: &str) {
    let name = "my-guard";
    let info = r#"{"description":"guard"}"#;
    let status = "pending_review";
    let client = pool.get().await.unwrap();
    client
        .execute(INSERT_SQL, &[&id, &name, &params, &info, &status])
        .await
        .unwrap();
}

#[test]
fn storage_binds_the_config_sync_table() {
    assert_eq!(ReviewRowsBinding::NAME, "litellm_guardrailstable");
    assert_eq!(ReviewRowsBinding::TABLE, "LiteLLM_GuardrailsTable");
}

fn review_rows<'db>(
    store: &'db PostgresStore,
    availability: &'db Availability,
    journal: &Arc<Mutex<Vec<String>>>,
) -> PostgresReviewRows<'db> {
    PostgresReviewRows {
        client: Some(PrismaClient {
            writer: store,
            reader: store,
            availability,
        }),
        publish: publisher(journal),
    }
}

fn assert_pending_row(row: &ReviewRow) {
    assert_eq!(row.guardrail_id, "g1");
    assert_eq!(row.guardrail_name, "my-guard");
    assert_eq!(row.status, "pending_review");
    assert!(row.litellm_params.is_some());
    assert!(row.guardrail_info.is_some());
    assert!(row.submitted_at.is_some());
    assert!(row.reviewed_at.is_none());
    assert_eq!(row.team_id, None);
}

#[tokio::test]
async fn storage_round_trips_a_review_row() {
    let (fixture, pool) = database().await;
    insert_row(&pool, "g1", r#"{"guardrail":"bedrock","mode":"pre_call"}"#).await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let rows = review_rows(&store, &availability, &journal);
    let row = rows
        .find_unique("g1")
        .await
        .expect("the read succeeds")
        .expect("the row exists");
    assert_pending_row(&row);
    assert!(journal.lock().unwrap().is_empty());
    rows.set_status("g1", "active", Utc::now())
        .await
        .expect("the write succeeds");
    assert_eq!(
        journal.lock().unwrap().clone(),
        vec!["litellm_guardrailstable".to_owned()]
    );
    let updated = rows
        .find_unique("g1")
        .await
        .expect("the read succeeds")
        .expect("the row exists");
    assert_eq!(updated.status, "active");
    assert!(updated.reviewed_at.is_some());
    assert!(updated.submitted_at.is_some());
    let missing = rows.set_status("absent", "active", Utc::now()).await;
    assert!(matches!(missing, Err(ReviewRowsError::MissingRow { .. })));
    drop(rows);
    pool.close();
    fixture.close_async().await.unwrap();
}
