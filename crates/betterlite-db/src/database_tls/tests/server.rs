use clap::Parser as _;
use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{ContainerAsync, GenericImage, ImageExt as _, runners::AsyncRunner as _};

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    postgres_test_url: Option<String>,
}

pub(super) struct Server {
    pub(super) url: String,
    _container: Option<ContainerAsync<GenericImage>>,
}

// POSTGRES_TEST_URL when the dispatcher or CI supplies a server, otherwise a testcontainers Postgres.
pub(super) async fn start() -> Server {
    let configured = Environment::parse().postgres_test_url;
    match configured {
        Some(url) if url.is_empty() => panic!("POSTGRES_TEST_URL is set but empty"),
        Some(url) => Server {
            url,
            _container: None,
        },
        None => start_container().await,
    }
}

async fn start_container() -> Server {
    let image = GenericImage::new("postgres", "17")
        .with_wait_for(WaitFor::message_on_stdout("database system is ready"))
        .with_wait_for(WaitFor::seconds(2))
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_USER", "fixture")
        .with_env_var("POSTGRES_PASSWORD", "fixture")
        .with_env_var("POSTGRES_DB", "postgres");
    let container = image.start().await.expect("a Postgres container");
    let host = container.get_host().await.expect("a container host");
    let port = container
        .get_host_port_ipv4(5432.tcp())
        .await
        .expect("a mapped container port");
    Server {
        url: format!("postgresql://fixture:fixture@{host}:{port}/postgres"),
        _container: Some(container),
    }
}
