#[cfg(test)]
mod ptu_job {
    use std::collections::BTreeMap;
    use std::error::Error as StdError;
    use std::future::{Future, ready};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use betterlite_db::budget_permission_rows::JsonData;
    use betterlite_db::database_failures::metrics::WallClock;
    use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
    use betterlite_db::postgres_store::session::PostgresStore;
    use betterlite_db::ptu::rollup::RollupError;
    use betterlite_db::ptu::rollup::alert::OperatorAlert;
    use betterlite_db::ptu::rollup::scheduled::{PTU_ROLLUP_JOB_ID, PTU_ROLLUP_LOCK_TTL_SECONDS};
    use betterlite_db::ptu::store::StoreError;
    use betterlite_db::ptu::store::sleeper::{Sleeper, TokioSleeper};
    use betterlite_db::queues::pod_lock::LockCache as _;
    use betterlite_db::queues::pod_lock::cache::RedisLockCache;
    use betterlite_db::queues::pod_lock::owner::{PodId, lease_key};
    use betterlite_db::queues::pod_lock::{NoPodLockEventSink, PodLockManager};
    use betterlite_models::model_registration::JsonObject;
    use betterlite_openai::openai_base_config::Object as JsonMap;
    use betterlite_postgres_fixture::connection::PostgresFixture;
    use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
    use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
    use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
    use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
    use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
    use betterlite_redis::redis_client_reuse::store::RedisClientStore;
    use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
    use betterlite_redis::redis_connection_settings::model::scalars::FloatOrText;
    use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
    use betterlite_redis::redis_live_client::LiveClientFactory;
    use betterlite_router::deployment_identity::spec::DeploymentSpec;
    use betterlite_server::failure_hook::alerts::ProxyAlerts;
    use betterlite_server::failure_hook::ports::{
        AlertLevel, AlertType, AlertingClient, BudgetAlertKind, CallInfo, Clock as AlertClock,
        FailureService, ServiceLogging, SlackAlert, SlackAlerting, UsageCache,
    };
    use betterlite_server::jobs::ptu::attribution::PtuAttribution;
    use betterlite_server::jobs::ptu::snapshot::CatalogSnapshot;
    use betterlite_server::jobs::ptu::task::{PtuRollupInputs, ScheduledPtuRollup};
    use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
    use clap::Parser as _;
    use deadpool_postgres::{Client, Manager, Pool};
    use futures_util::future::BoxFuture;
    use rust_decimal::Decimal;
    use secrecy::{ExposeSecret as _, SecretString};
    use tempfile::TempDir;
    use tokio::net::UnixStream;
    use tokio::process::{Child, Command};
    use tokio::task::yield_now;
    use tokio::time::sleep;
    use tokio_postgres::NoTls;
    use tokio_postgres::types::Json;
    use tokio_util::task::TaskTracker;
    use uuid::Uuid;

    #[derive(clap::Parser)]
    struct Environment {
        #[arg(long, env = "POSTGRES_TEST_URL")]
        url: Option<String>,
    }

    struct Database {
        fixture: PostgresFixture,
        pool: Pool,
    }

    impl Database {
        async fn start() -> Self {
            let url = Environment::parse_from(["ptu-job-tests"])
                .url
                .map(SecretString::from);
            let fixture = PostgresFixture::provision_with_statements(
                url.as_ref(),
                catalog_statements(&PACKAGED_CATALOGS).unwrap(),
            )
            .resolve()
            .await
            .unwrap();
            let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
            let pool = Pool::builder(manager).max_size(3).build().unwrap();
            Self { fixture, pool }
        }

        async fn client(&self) -> Client {
            self.pool.get().await.unwrap()
        }

        fn store(&self) -> PostgresStore {
            PostgresStore {
                pool: Some(self.pool.clone()),
            }
        }

        async fn close(self) {
            self.pool.close();
            yield_now().await;
            self.fixture.close_async().await.unwrap();
        }
    }

    struct SilentHooks;

    impl ServiceHooks for SilentHooks {
        fn success(&self, _event: &ServiceEvent) {}

        fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
    }

    struct ClosedGate;

    impl BreakerGate for ClosedGate {
        fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
            Ok(BreakerAdmission::from(0_u64))
        }

        fn exit(&self, _admission: BreakerAdmission) {}

        fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

        fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
    }

    async fn lock_cache(url: String) -> RedisLockCache {
        let overrides = RedisOverrides {
            url: Some(Some(SecretString::from(url))),
            socket_timeout: Some(FloatOrText::Value(1.0)),
            socket_connect_timeout: Some(FloatOrText::Value(1.0)),
            ..RedisOverrides::default()
        };
        let options = RedisCacheOptions {
            overrides,
            family: RedisCacheFamily::Standalone,
            namespace: None,
            default_ttl: 60_i64,
            redis_flush_size: None,
            default_major_version: 8_u32,
            factory: Arc::new(LiveClientFactory::new(None)),
            store: Arc::new(RedisClientStore::new()),
            hooks: Arc::new(SilentHooks),
            gate: Arc::new(ClosedGate),
        };
        let environment: BTreeMap<String, String> = BTreeMap::new();
        let cache = connect(options, &environment).await.unwrap();
        RedisLockCache {
            cache: Arc::new(cache),
        }
    }

    // One proxy per process owns one UUID: every manager built here is a different pod.
    fn lock_manager(cache: Arc<RedisLockCache>) -> PodLockManager<RedisLockCache> {
        PodLockManager::new(
            Some(cache),
            PTU_ROLLUP_LOCK_TTL_SECONDS,
            Arc::new(NoPodLockEventSink),
            PodId::try_from(Uuid::new_v4()).unwrap(),
        )
    }

    async fn lease_held(cache: &RedisLockCache) -> bool {
        cache
            .get_owner(&lease_key(PTU_ROLLUP_JOB_ID))
            .await
            .unwrap()
            .is_some()
    }

    struct Redis {
        _child: Child,
        socket: PathBuf,
        _directory: TempDir,
    }

    impl Redis {
        async fn start() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("redis.sock");
            let child = Command::new("redis-server")
                .arg("--port")
                .arg("0")
                .arg("--unixsocket")
                .arg(&socket)
                .arg("--save")
                .arg("")
                .arg("--appendonly")
                .arg("no")
                .arg("--dir")
                .arg(directory.path())
                .arg("--logfile")
                .arg(directory.path().join("redis.log"))
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            wait_for_redis(&socket).await;
            Self {
                _child: child,
                socket,
                _directory: directory,
            }
        }

        fn url(&self) -> String {
            format!("redis+unix://{}", self.socket.display())
        }
    }

    async fn wait_for_redis(socket: &Path) {
        for _attempt in 0..500_u32 {
            if UnixStream::connect(socket).await.is_ok() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("redis-server never answered on {}", socket.display());
    }

    struct FixedClock(DateTime<Utc>);

    impl WallClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[derive(Default)]
    struct NoSleep;

    impl Sleeper for NoSleep {
        fn sleep(&mut self, _duration: Duration) -> impl Future<Output = ()> + Send {
            ready(())
        }
    }

    struct AlwaysOn;

    impl PtuAttribution for AlwaysOn {
        fn enabled(&self) -> bool {
            true
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("the test has no alert sink")]
    struct NoAlertError;

    struct NoAlert;

    impl OperatorAlert for NoAlert {
        type Error = NoAlertError;

        fn send<'message>(
            &'message self,
            _message: &'message str,
        ) -> BoxFuture<'message, Result<(), NoAlertError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Default)]
    struct NoUsage;

    #[async_trait]
    impl UsageCache for NoUsage {
        async fn set_local(&self, _key: &str, _value: &str, _ttl: Duration) {}
    }

    struct NoLogging;

    #[async_trait]
    impl ServiceLogging for NoLogging {
        async fn async_service_failure_hook(
            &self,
            _service: FailureService,
            _duration: Duration,
            _error: &str,
            _call_type: &str,
        ) {
        }
    }

    struct FixedAlertClock;

    impl AlertClock for FixedAlertClock {
        fn now(&self) -> DateTime<FixedOffset> {
            DateTime::parse_from_rfc3339("2026-03-01T00:15:00Z").unwrap()
        }
    }

    #[derive(Default)]
    struct RecordingSlack {
        sent: Mutex<Vec<(AlertLevel, AlertType, String)>>,
    }

    #[async_trait]
    impl SlackAlerting for RecordingSlack {
        fn alerting_configured(&self) -> bool {
            true
        }

        async fn failed_tracking_alert(&self, _error_message: &str, _failing_model: &str) {}

        async fn budget_alerts(&self, _kind: BudgetAlertKind, _user_info: &CallInfo) {}

        async fn send_alert(&self, alert: SlackAlert<'_>) {
            self.sent.lock().unwrap().push((
                alert.level,
                alert.alert_type,
                alert.message.to_owned(),
            ));
        }

        async fn response_taking_too_long(&self, _data: Option<JsonMap>) {}
    }

    // Slack records the alert while the Sentry client has no DSN, so its delivery fails inside alerting_handler.
    fn failing_sentry_alerts(slack: Arc<RecordingSlack>) -> ProxyAlerts {
        ProxyAlerts {
            alerting: Some(vec![AlertingClient::Slack, AlertingClient::Sentry]),
            alert_types: Vec::new(),
            alerting_threshold: Duration::ZERO,
            proxy_base_url: None,
            usage_cache: Arc::new(NoUsage),
            slack,
            email: None,
            service_logging: Arc::new(NoLogging),
            sentry: None,
            redactor: None,
            clock: Arc::new(FixedAlertClock),
            tasks: TaskTracker::new(),
        }
    }

    fn job<A: OperatorAlert>(
        database: &Database,
        lock: PodLockManager<RedisLockCache>,
        deployments: Arc<CatalogSnapshot>,
        alerts: Option<Arc<A>>,
    ) -> Arc<ScheduledPtuRollup<RedisLockCache, A>> {
        Arc::new(ScheduledPtuRollup::new(PtuRollupInputs {
            store: database.store(),
            deployments,
            lock: Arc::new(lock),
            alerts,
            attribution: Arc::new(AlwaysOn),
            clock: Arc::new(FixedClock(
                DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )),
        }))
    }

    struct Running<A: OperatorAlert> {
        job: Arc<ScheduledPtuRollup<RedisLockCache, A>>,
        cache: Arc<RedisLockCache>,
        _redis: Redis,
    }

    async fn running_job(
        database: &Database,
        deployments: Arc<CatalogSnapshot>,
    ) -> Running<NoAlert> {
        running_job_with_alerts(database, deployments, None).await
    }

    async fn running_job_with_alerts<A: OperatorAlert>(
        database: &Database,
        deployments: Arc<CatalogSnapshot>,
        alerts: Option<Arc<A>>,
    ) -> Running<A> {
        let redis = Redis::start().await;
        let cache = Arc::new(lock_cache(redis.url()).await);
        let job = job(
            database,
            lock_manager(Arc::clone(&cache)),
            deployments,
            alerts,
        );
        Running {
            job,
            cache,
            _redis: redis,
        }
    }

    async fn seed_database() -> (Database, Client) {
        let database = Database::start().await;
        let client = database.client().await;
        (database, client)
    }

    async fn seed_ptu_model(client: &Client, model: &str, team: &str, from: &str) {
        insert_model(
            client,
            model,
            &format!("name-{model}"),
            &ptu_info(team, from),
        )
        .await;
    }

    async fn seed_ended_model(client: &Client, model: &str, team: &str, from: &str, to: &str) {
        insert_model(
            client,
            model,
            &format!("name-{model}"),
            &ended_info(team, from, to),
        )
        .await;
    }

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn ptu_info(team: &str, from: &str) -> JsonData {
        serde_json::from_str(&format!(
            r#"{{"ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "{team}", "ptu_effective_from": "{from}"}}"#
        ))
        .unwrap()
    }

    fn ended_info(team: &str, from: &str, to: &str) -> JsonData {
        serde_json::from_str(&format!(
            r#"{{"ptu_count": 1, "cost_per_ptu_per_hour": 1, "team_id": "{team}", "ptu_effective_from": "{from}", "ptu_effective_to": "{to}"}}"#
        ))
        .unwrap()
    }

    fn deployment(model_name: &str, team: &str, from: &str) -> DeploymentSpec {
        let text = serde_json::to_string(&ptu_info(team, from)).unwrap();
        let model_info: JsonObject = serde_json::from_str(&text).unwrap();
        DeploymentSpec::new(
            model_name.to_owned(),
            JsonObject::new(),
            model_info,
            JsonObject::new(),
            || "cfg-id".to_owned(),
        )
        .unwrap()
    }

    async fn insert_model(client: &Client, id: &str, name: &str, info: &JsonData) {
        let info = Json(info);
        client
            .execute(
                "INSERT INTO \"LiteLLM_ProxyModelTable\" \
                 (\"model_id\", \"model_name\", \"litellm_params\", \"model_info\", \
                  \"created_by\", \"updated_by\") \
                 VALUES ($1, $2, '{}'::jsonb, $3, 't', 't')",
                &[&id, &name, &info],
            )
            .await
            .unwrap();
    }

    async fn seed_sentinel(client: &Client, day: &str, model: &str, cost: &str) {
        client
            .batch_execute(&format!(
                "INSERT INTO \"LiteLLM_DailyTeamSpend\" \
                 (\"id\", \"team_id\", \"date\", \"api_key\", \"model\", \"model_group\", \
                  \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\", \"spend\", \
                  \"api_requests\", \"prompt_tokens\", \"created_at\", \"updated_at\", \"ptu_flat_cost\") \
                 VALUES ('stale', 'team-1', '{day}', '__ptu_flat_cost__', '{model}', '{model}', \
                  '', '', '', 0, 0, 0, now(), '{day} 11:00:00', {cost})"
            ))
            .await
            .unwrap();
    }

    async fn cost_of(client: &Client, day: &str, model: &str) -> Option<Decimal> {
        let row = client
            .query_opt(
                "SELECT \"ptu_flat_cost\" FROM \"LiteLLM_DailyTeamSpend\" \
                 WHERE \"date\" = $1 AND \"model\" = $2",
                &[&day, &model],
            )
            .await
            .unwrap()?;
        Some(row.get(0))
    }

    // Every attempt for one model fails, so both the daily pass and the catch-up pass report its charge as lost.
    async fn failing_upserts(client: &Client, model: &str) {
        client
            .batch_execute(&format!(
                "CREATE SEQUENCE ptu_upsert_attempts; \
                 CREATE FUNCTION ptu_flaky_upsert() RETURNS trigger LANGUAGE plpgsql AS $$ \
                 BEGIN \
                   IF NEW.model = '{model}' THEN \
                     IF nextval('ptu_upsert_attempts') <= 1000 THEN \
                       RAISE EXCEPTION 'transient upsert failure'; \
                     END IF; \
                   END IF; \
                   RETURN NEW; \
                 END; \
                 $$; \
                 CREATE TRIGGER ptu_flaky_upsert_trigger BEFORE INSERT OR UPDATE \
                 ON \"LiteLLM_DailyTeamSpend\" FOR EACH ROW EXECUTE FUNCTION ptu_flaky_upsert();"
            ))
            .await
            .unwrap();
    }

    async fn slow_upserts(client: &Client, model: &str) {
        client
            .batch_execute(&format!(
                "CREATE FUNCTION ptu_slow_upsert() RETURNS trigger LANGUAGE plpgsql AS $$ \
                 BEGIN \
                   IF NEW.model = '{model}' THEN \
                     PERFORM pg_sleep(5); \
                   END IF; \
                   RETURN NEW; \
                 END; \
                 $$; \
                 CREATE TRIGGER ptu_slow_upsert_trigger BEFORE INSERT OR UPDATE \
                 ON \"LiteLLM_DailyTeamSpend\" FOR EACH ROW EXECUTE FUNCTION ptu_slow_upsert();"
            ))
            .await
            .unwrap();
    }

    async fn wait_for_row(client: &Client, day: &str, model: &str) {
        for _attempt in 0..500_u32 {
            if cost_of(client, day, model).await.is_some() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("the charge for {model} on {day} never landed");
    }

    #[tokio::test]
    async fn a_held_lease_skips_the_day() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        let running = running_job(&database, Arc::new(CatalogSnapshot::default())).await;
        let other = lock_manager(Arc::clone(&running.cache));
        assert_eq!(
            other.acquire_lock(PTU_ROLLUP_JOB_ID, Some(900), true).await,
            Some(true)
        );
        assert!(running.job.run::<NoSleep>().await.unwrap().is_none());
        assert!(cost_of(&client, "2026-02-28", "model-a").await.is_none());
        drop(client);
        database.close().await;
        drop(running);
    }

    #[tokio::test]
    async fn an_expired_lease_lets_the_run_through() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        let running = running_job(&database, Arc::new(CatalogSnapshot::default())).await;
        let other = lock_manager(Arc::clone(&running.cache));
        assert_eq!(
            other.acquire_lock(PTU_ROLLUP_JOB_ID, Some(1), true).await,
            Some(true)
        );
        sleep(Duration::from_millis(1_100)).await;
        let result = running
            .job
            .run::<NoSleep>()
            .await
            .unwrap()
            .expect("the expired lease lets the run through");
        assert_eq!(result.day, date("2026-02-28"));
        assert_eq!(result.rows_written, 1);
        assert_eq!(
            cost_of(&client, "2026-02-28", "model-a").await,
            Some(Decimal::from(24_i32))
        );
        assert!(!lease_held(&running.cache).await);
        drop(client);
        database.close().await;
        drop(running);
    }

    #[tokio::test]
    async fn an_unreadable_redis_runs_the_day_without_pruning() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        seed_ended_model(
            &client,
            "model-z",
            "team-1",
            "2026-02-20T00:00:00Z",
            "2026-02-24T00:00:00Z",
        )
        .await;
        seed_sentinel(&client, "2026-02-28", "model-z", "3.5").await;
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "redis+unix://{}",
            directory.path().join("absent.sock").display()
        );
        let cache = Arc::new(lock_cache(url).await);
        let job = job(
            &database,
            lock_manager(Arc::clone(&cache)),
            Arc::new(CatalogSnapshot::default()),
            None,
        );
        let result = job
            .run::<NoSleep>()
            .await
            .unwrap()
            .expect("an unreadable lease runs the day unguarded");
        assert_eq!(result.rows_written, 1);
        assert_eq!(
            cost_of(&client, "2026-02-28", "model-a").await,
            Some(Decimal::from(24_i32))
        );
        assert_eq!(
            cost_of(&client, "2026-02-28", "model-z").await,
            Some(Decimal::new(35, 1))
        );
        drop(client);
        database.close().await;
    }

    #[tokio::test]
    async fn a_deployment_outside_the_stored_catalog_is_priced() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        let specs = [deployment("cfg-name", "team-2", "2026-02-26T00:00:00Z")];
        let running = running_job(&database, Arc::new(CatalogSnapshot::from_specs(&specs))).await;
        let result = running
            .job
            .run::<NoSleep>()
            .await
            .unwrap()
            .expect("the enabled job runs");
        assert_eq!(result.models_processed, 2);
        assert_eq!(result.rows_written, 2);
        assert_eq!(
            cost_of(&client, "2026-02-28", "model-a").await,
            Some(Decimal::from(24_i32))
        );
        assert_eq!(
            cost_of(&client, "2026-02-28", "cfg-id").await,
            Some(Decimal::from(24_i32))
        );
        drop(client);
        database.close().await;
        drop(running);
    }

    #[tokio::test]
    async fn a_scan_failure_stops_the_day_and_releases_the_lease() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        client
            .batch_execute("DROP TABLE \"LiteLLM_ProxyModelTable\"")
            .await
            .unwrap();
        let running = running_job(&database, Arc::new(CatalogSnapshot::default())).await;
        let outcome = running.job.run::<NoSleep>().await;
        assert!(matches!(
            outcome,
            Err(RollupError::Store(StoreError::Scan(_)))
        ));
        assert!(!lease_held(&running.cache).await);
        drop(client);
        database.close().await;
        drop(running);
    }

    #[tokio::test]
    async fn a_failed_charge_keeps_the_landed_rows_and_the_daily_result() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        seed_ptu_model(&client, "model-b", "team-1", "2026-02-26T00:00:00Z").await;
        failing_upserts(&client, "model-a").await;
        let running = running_job(&database, Arc::new(CatalogSnapshot::default())).await;
        let result = running
            .job
            .run::<NoSleep>()
            .await
            .unwrap()
            .expect("the day's result survives a failed charge");
        assert_eq!(result.rows_written, 1);
        assert_eq!(result.rows_failed, 1);
        assert!(cost_of(&client, "2026-02-28", "model-a").await.is_none());
        assert_eq!(
            cost_of(&client, "2026-02-28", "model-b").await,
            Some(Decimal::from(24_i32))
        );
        drop(client);
        database.close().await;
        drop(running);
    }

    #[tokio::test]
    async fn a_failing_alert_channel_keeps_the_daily_result() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        seed_ptu_model(&client, "model-b", "team-1", "2026-02-26T00:00:00Z").await;
        failing_upserts(&client, "model-a").await;
        let slack = Arc::new(RecordingSlack::default());
        let alerts = failing_sentry_alerts(Arc::clone(&slack));
        let running = running_job_with_alerts(
            &database,
            Arc::new(CatalogSnapshot::default()),
            Some(Arc::new(alerts)),
        )
        .await;
        let result = running
            .job
            .run::<NoSleep>()
            .await
            .unwrap()
            .expect("the day's result survives a failing alert channel");
        assert_eq!(result.rows_written, 1);
        assert_eq!(result.rows_failed, 1);
        let sent = slack.sent.lock().unwrap().clone();
        let daily = sent
            .iter()
            .find(|(_, _, message)| message.contains("rollup for 2026-02-28"))
            .expect("the daily failed-charge alert reached Slack despite the Sentry error");
        assert_eq!(daily.0, AlertLevel::High);
        assert_eq!(daily.1, AlertType::FailedTrackingSpend);
        assert!(
            daily.2.contains("1 of 2 team charges failed to write"),
            "{:?}",
            daily.2
        );
        drop(client);
        database.close().await;
        drop(running);
    }

    #[tokio::test]
    async fn a_cancelled_run_keeps_the_rows_it_landed() {
        let (database, client) = seed_database().await;
        seed_ptu_model(&client, "model-a", "team-1", "2026-02-26T00:00:00Z").await;
        seed_ptu_model(&client, "model-b", "team-1", "2026-02-26T00:00:00Z").await;
        slow_upserts(&client, "model-b").await;
        let running = running_job(&database, Arc::new(CatalogSnapshot::default())).await;
        let job = Arc::clone(&running.job);
        let tracker = TaskTracker::new();
        let spawned = tracker.spawn(async move { job.run::<TokioSleeper>().await });
        wait_for_row(&client, "2026-02-28", "model-a").await;
        spawned.abort();
        let joined = spawned.await;
        assert!(tracker.close());
        tracker.wait().await;
        assert!(joined.unwrap_err().is_cancelled());
        assert_eq!(
            cost_of(&client, "2026-02-28", "model-a").await,
            Some(Decimal::from(24_i32))
        );
        assert!(lease_held(&running.cache).await);
        drop(client);
        database.close().await;
        drop(running);
    }
}
