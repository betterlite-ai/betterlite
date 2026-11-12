use std::sync::Arc;
use std::time::Duration;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::{sleep, timeout};
use tokio_postgres::NoTls;

use crate::database_refresh::access::RefreshTrigger as _;
use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::handle::{PoolHandle as _, PoolProvider as _};
use crate::database_refresh::native::{NativePool, NativePoolProvider};
use crate::database_refresh::replacement::ReplacementError;
use crate::database_refresh_loop::endpoint::WriterEndpoint;
use crate::database_refresh_loop::lifecycle::{DatabaseRefreshLoop, RefreshLoopConfig};
use crate::database_refresh_loop::minting::TokenMinter;
use crate::database_refresh_loop::port::RefreshError;
use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_url_policy::url_codec::urlsplit;

use super::fakes::{FakeError, lock, secret, slot_url, utc};
use super::loops::{FakeCallback, fixed_clock, options};

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database_fixture() -> PostgresFixture {
    let configured = Environment::parse_from(["database-refresh-loop-tests"])
        .url
        .map(SecretString::from);
    let mut task = PostgresFixture::provision_with_name_generator(
        configured.as_ref(),
        &FixtureSeed::default(),
        &mut RandomDatabaseName,
    );
    task.resolve().await.expect("a Postgres database fixture")
}

fn credentials(url: &str) -> (String, String, String, String, String) {
    let parsed = urlsplit(url).expect("the fixture URL parses");
    let (userinfo, hostport) = parsed
        .netloc
        .rsplit_once('@')
        .expect("the fixture URL carries credentials");
    let (user, password) = userinfo
        .split_once(':')
        .expect("the fixture URL carries a password");
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "5432"));
    (
        host.to_owned(),
        port.to_owned(),
        user.to_owned(),
        parsed.path.trim_start_matches('/').to_owned(),
        password.to_owned(),
    )
}

pub(crate) fn writer_endpoint_from(url: &str) -> WriterEndpoint {
    let (host, port, user, name, _password) = credentials(url);
    WriterEndpoint {
        host: Some(host),
        port,
        user: Some(user),
        name: Some(name),
        schema: None,
    }
}

pub(crate) fn raw_password(url: &str) -> SecretString {
    let (_host, _port, _user, _name, password) = credentials(url);
    secret(&password)
}

pub(crate) struct PasswordMinter {
    pub(crate) password: SecretString,
}

impl TokenMinter for PasswordMinter {
    type Error = FakeError;

    fn token(&self, _endpoint: &IAMEndpoint) -> Result<Option<SecretString>, FakeError> {
        Ok(Some(self.password.clone()))
    }
}

pub(crate) fn fixture_pool(url: &SecretString) -> Pool {
    let config = url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses for a plain pool");
    Pool::builder(Manager::new(config, NoTls))
        .max_size(1)
        .build()
        .expect("the fixture pool builds")
}

pub(crate) async fn row_count(pool: &Pool, table: &str) -> i64 {
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .expect("the row count query runs");
    row.get(0)
}

pub(crate) async fn close_fixture(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("fixture cleanup");
}

pub(crate) async fn closed(handle: &NativePool) {
    let settled = timeout(Duration::from_secs(5), async {
        while handle.is_connected() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    settled.expect("the retired pool closes");
}

pub(crate) struct PostgresCase {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
    pub(crate) url: SecretString,
}

pub(crate) struct FixtureLoop {
    pub(crate) case: PostgresCase,
    pub(crate) initial: Arc<NativePool>,
    pub(crate) slot: DatabaseUrlSlot,
    pub(crate) built: DatabaseRefreshLoop<NativePoolProvider, PasswordMinter, FakeError>,
}

pub(crate) async fn seeded_case() -> PostgresCase {
    let fixture = database_fixture().await;
    let url = fixture.options.url.clone();
    let pool = fixture_pool(&url);
    let seed = pool.get().await.expect("a fixture connection");
    seed.execute("CREATE TABLE refresh_rows (note text)", &[])
        .await
        .expect("the table is created");
    seed.execute("INSERT INTO refresh_rows VALUES ('pinned')", &[])
        .await
        .expect("the row commits");
    drop(seed);
    PostgresCase { fixture, pool, url }
}

pub(crate) async fn loop_on(
    case: PostgresCase,
    endpoint: WriterEndpoint,
    on_replaced: Option<FakeCallback>,
) -> FixtureLoop {
    let provider = Arc::new(NativePoolProvider);
    let initial = Arc::new(
        provider
            .build(&case.url, None)
            .expect("the initial native pool builds"),
    );
    let slot = DatabaseUrlSlot::default();
    drop(slot.publish(Some(case.url.clone())));
    let built = DatabaseRefreshLoop::<NativePoolProvider, PasswordMinter, FakeError>::new(
        provider,
        options(slot.clone()),
        Some(Arc::clone(&initial)),
        RefreshLoopConfig {
            writer_endpoint: endpoint,
            minter: PasswordMinter {
                password: raw_password(case.url.expose_secret()),
            },
            clock: fixed_clock(),
        },
        on_replaced,
    )
    .expect("the refresh loop builds");
    built
        .generation
        .connect(None)
        .await
        .expect("the initial pool connects");
    FixtureLoop {
        case,
        initial,
        slot,
        built,
    }
}

pub(crate) async fn close_case(setup: FixtureLoop) {
    setup.built.generation.shutdown().await;
    close_fixture(setup.case.fixture, setup.case.pool).await;
}

pub(crate) async fn assert_published(setup: &FixtureLoop) {
    let observed = setup.built.generation.observe();
    assert_eq!(observed.generation, 1);
    assert!(observed.connected);
    let fresh = observed.handle.expect("the new generation is published");
    let client = fresh.session().await.expect("the new generation connects");
    let row = client
        .query_one("SELECT count(*) FROM refresh_rows", &[])
        .await
        .expect("the pinned row reads from the new pool");
    drop(client);
    assert_eq!(row.get::<_, i64>(0), 1);
    assert_eq!(row_count(&setup.case.pool, "refresh_rows").await, 1);
    assert_ne!(
        slot_url(&setup.slot),
        Some(setup.case.url.expose_secret().to_owned())
    );
    assert_eq!(
        *lock(&setup.built.generation.last_refresh_time),
        Some(utc("2024-01-01T00:00:00"))
    );
}

pub(crate) async fn candidate_failure_case() {
    let case = seeded_case().await;
    let mut unreachable = writer_endpoint_from(case.url.expose_secret());
    unreachable.port = "1".to_owned();
    let setup = loop_on(case, unreachable, None).await;
    let before = slot_url(&setup.slot);
    let outcome = setup.built.port.refresh().await;
    assert!(matches!(
        outcome,
        Err(RefreshError::Replace {
            source: ReplacementError::Connect { .. }
        })
    ));
    assert_eq!(slot_url(&setup.slot), before);
    assert_eq!(setup.built.generation.observe().generation, 0);
    assert!(setup.built.generation.observe().connected);
    assert_eq!(row_count(&setup.case.pool, "refresh_rows").await, 1);
    let client = setup
        .initial
        .session()
        .await
        .expect("the old generation keeps serving");
    let row = client
        .query_one("SELECT count(*) FROM refresh_rows", &[])
        .await
        .expect("the old pool still reads the pinned row");
    drop(client);
    assert_eq!(row.get::<_, i64>(0), 1);
    Box::pin(close_case(setup)).await;
}

pub(crate) async fn callback_error_case() {
    let case = seeded_case().await;
    let endpoint = writer_endpoint_from(case.url.expose_secret());
    let setup = loop_on(case, endpoint, Some(Arc::new(|| Err(FakeError::Callback)))).await;
    let before = slot_url(&setup.slot);
    let outcome = setup.built.port.refresh().await;
    assert!(matches!(
        outcome,
        Err(RefreshError::Replace {
            source: ReplacementError::Callback { .. }
        })
    ));
    assert_eq!(slot_url(&setup.slot), before);
    let observed = setup.built.generation.observe();
    assert_eq!(observed.generation, 1);
    assert!(observed.connected);
    let fresh = observed.handle.expect("the new generation stays live");
    let client = fresh.session().await.expect("the new generation connects");
    client
        .execute("INSERT INTO refresh_rows VALUES ('after swap')", &[])
        .await
        .expect("the new generation writes");
    drop(client);
    assert_eq!(row_count(&setup.case.pool, "refresh_rows").await, 2);
    assert_eq!(*lock(&setup.built.generation.last_refresh_time), None);
    Box::pin(close_case(setup)).await;
}
