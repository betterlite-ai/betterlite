use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser as _;
use secrecy::ExposeSecret as _;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Server {
    pub(super) url: String,
    fixture: PostgresFixture,
}

// parse_from keeps nextest's argv out of clap
pub(super) async fn start() -> Server {
    let configured = Environment::parse_from(["database-tls-tests"])
        .url
        .map(secrecy::SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(configured.as_ref(), Vec::new())
        .resolve()
        .await
        .expect("a Postgres fixture");
    let url = fixture.options.url.expose_secret().to_owned();
    Server { url, fixture }
}

impl Server {
    pub(super) async fn close(self) {
        self.fixture.close_async().await.expect("fixture cleanup");
    }
}
