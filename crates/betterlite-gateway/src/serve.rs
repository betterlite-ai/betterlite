use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::chatgpt_token_store::home::HomeEnvironment;
use betterlite_config::sections::includes::load_file;
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_server::server::ServerError;
use betterlite_server::server::chat_setup::chat_executor;
use betterlite_server::server::run_with;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tokio::runtime::Builder;

#[derive(Clone, Copy)]
pub struct ComponentRoutes {
    pub prefixes: &'static [&'static str],
    pub exact: &'static phf::Set<&'static str>,
    pub mounts: &'static phf::Set<&'static str>,
}

#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}

// LiteLLM keeps a route when it is a kept mount, an exact path, or under a kept prefix
pub fn matches_path(path: &str, routes: ComponentRoutes) -> bool {
    routes.mounts.contains(path)
        || routes.exact.contains(path)
        || routes
            .prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

async fn filter_path(
    State(routes): State<ComponentRoutes>,
    request: Request,
    next: Next,
) -> Response {
    if matches_path(request.uri().path(), routes) {
        next.run(request).await
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(NotFoundBody {
                detail: "Not Found",
            }),
        )
            .into_response()
    }
}

pub fn serve_component(
    config_path: PathBuf,
    host: String,
    port: u16,
    routes: ComponentRoutes,
) -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let environment = BTreeMap::new();
    let Some(config) = load_config(&config_path, environment.clone()) else {
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
    match runtime.block_on(serve(config, chat, &host, port, routes)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server startup failed");
            ExitCode::from(2)
        }
    }
}

async fn serve(
    config: Config,
    chat: ChatExecutor,
    host: &str,
    port: u16,
    routes: ComponentRoutes,
) -> Result<(), ServerError> {
    run_with(config, chat, host, port, |application| {
        application.layer(from_fn_with_state(routes, filter_path))
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
