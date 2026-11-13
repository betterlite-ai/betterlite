use super::chat_setup::chat_executor;
use super::filter::ComponentRoutes;
use super::run_with;
use axum::Extension;
use betterlite_config::loader::startup::{load_config, process_environment};
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tokio::runtime::{Builder, Runtime};

pub struct Startup {
    pub runtime: Runtime,
    pub config: Config,
    pub chat: ChatExecutor,
}

// betterlite and the component binaries start the same way; a failure is logged and returns None
#[must_use]
pub fn startup(config_path: &Path) -> Option<Startup> {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let environment = process_environment();
    let config = load_config(config_path, environment.clone())?;
    let random = match StdRng::try_from_rng(&mut SysRng) {
        Ok(random) => random,
        Err(error) => {
            tracing::error!(error = %error, "failed to seed the deployment picker from the OS");
            return None;
        }
    };
    let chat = match chat_executor(&config, &environment, random) {
        Ok(chat) => chat,
        Err(error) => {
            tracing::error!(error = %error, "failed to set up chat completions");
            return None;
        }
    };
    match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => Some(Startup {
            runtime,
            config,
            chat,
        }),
        Err(error) => {
            tracing::error!(error = ?error, "failed to build runtime");
            None
        }
    }
}

// a component binary owns only its CLI defaults, its path lists and main; the serve runs here once
pub fn serve_component(
    config_path: PathBuf,
    host: String,
    port: u16,
    routes: ComponentRoutes,
) -> ExitCode {
    let Some(Startup {
        runtime,
        config,
        chat,
    }) = startup(&config_path)
    else {
        return ExitCode::FAILURE;
    };
    match runtime.block_on(run_with(config, chat, &host, port, |application| {
        application.layer(Extension(routes))
    })) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server startup failed");
            ExitCode::from(2)
        }
    }
}
