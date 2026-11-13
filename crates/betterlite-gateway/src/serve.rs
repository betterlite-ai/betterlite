use crate::cli::Cli;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::chatgpt_token_store::home::HomeEnvironment;
use betterlite_config::sections::includes::load_file;
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_server::server::ServerError;
use betterlite_server::server::chat_setup::chat_executor;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::Path;
use std::process::ExitCode;
use tokio::runtime::Builder;

pub fn run(cli: Cli) -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let environment = BTreeMap::new();
    let Some(config) = load_config(&cli.config, environment.clone()) else {
        return ExitCode::FAILURE;
    };
    let random = match StdRng::try_from_rng(&mut SysRng) {
        Ok(random) => random,
        Err(error) => {
            tracing::error!(error = %error, "failed to seed the deployment picker from the OS");
            return ExitCode::FAILURE;
        }
    };
    let chat = match chat_executor(&config, &environment, random) {
        Ok(executor) => executor,
        Err(error) => {
            tracing::error!(error = %error, "failed to set up chat completions");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(error = ?error, "failed to build runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(config, chat, &cli.host, cli.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}

async fn serve(config: Config, chat: ChatExecutor, host: &str, port: u16) -> Result<(), ServerError> {
    betterlite_server::server::run_with(config, chat, host, port, |router| {
        router.layer(axum::middleware::from_fn(crate::filter::stage))
    })
    .await
}

fn load_config(location: &Path, environment: BTreeMap<String, String>) -> Option<Config> {
    let loader = ConfigLoader {
        lookup: HomeEnvironment {
            variables: environment,
            home: env::home_dir(),
        },
    };
    match load_file(&loader, location) {
        Ok(config) => Some(config),
        Err(error) => {
            tracing::error!(path = %location.display(), error = ?error, "failed to read configuration");
            None
        }
    }
}
