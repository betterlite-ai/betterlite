use super::chat_setup::chat_executor;
use super::filter::ComponentRoutes;
use super::{application, serve};
use axum::Extension;
use betterlite_config::loader::startup::{load_config, process_environment};
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
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
    config_path: &Path,
    host: &str,
    port: u16,
    routes: ComponentRoutes,
) -> ExitCode {
    let Some(Startup {
        runtime,
        config,
        chat,
    }) = startup(config_path)
    else {
        return ExitCode::FAILURE;
    };
    let result = runtime.block_on(async {
        let shutting_down = Arc::new(AtomicBool::new(false));
        let outbound = chat.client.clone();
        let app = application(&config, chat, Arc::clone(&shutting_down))?.layer(Extension(routes));
        Box::pin(serve(app, shutting_down, outbound, host, port)).await
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server startup failed");
            ExitCode::from(2)
        }
    }
}
