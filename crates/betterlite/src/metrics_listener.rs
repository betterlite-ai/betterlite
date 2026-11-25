use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::net::SocketAddr;
use std::os::unix::process::parent_id;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::{self, ExitCode};
use std::time::Duration;

use axum::Router;
use betterlite_config::loader::startup::process_environment;
use betterlite_http::metrics_server::{
    MetricsServerError, STARTUP_POLLING, StartupProbe, build_metrics_app, wait_until_serving,
};
use betterlite_http::proxy::base_builder;
use clap::{Args, Command, Error, FromArgMatches as _};
use futures_util::future::{Either, select};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnectionBuilder;
use hyper_util::service::TowerToHyperService;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

// this file is also the bin root, where a plain mod tests; would look in src/tests.rs
#[path = "metrics_listener/tests.rs"]
mod tests;

const PROGRAM: &str = "betterlite-metrics-server";
const MULTIPROC_DIR: &str = "PROMETHEUS_MULTIPROC_DIR";
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const PARENT_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct MetricsListener {
    pub address: SocketAddr,
    pub serving: JoinHandle<io::Result<()>>,
}

// LiteLLM's prometheus_metrics_server CLI: --port has no default, and --multiproc_dir falls back to the environment
#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct MetricsServerArgs {
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    #[arg(long)]
    pub port: u16,
    #[arg(long = "multiproc_dir")]
    pub multiproc_dir: PathBuf,
}

// one connection: hyper serves it until it ends, and a stop lets the request in flight finish first
async fn serve_connection(
    stream: TcpStream,
    app: Router,
    stop: CancellationToken,
) -> io::Result<()> {
    // HTTP/1 only, as uvicorn serves it: the auto builder's HTTP/2 path spawns request tasks this JoinSet never owns
    let builder = ConnectionBuilder::new(TokioExecutor::new()).http1_only();
    let service = TowerToHyperService::new(app);
    let mut connection = pin!(builder.serve_connection(TokioIo::new(stream), service));
    match select(connection.as_mut(), pin!(stop.cancelled())).await {
        Either::Left((served, _)) => served.map_err(io::Error::other),
        Either::Right(((), mut draining)) => {
            draining.as_mut().graceful_shutdown();
            draining.as_mut().await.map_err(io::Error::other)
        }
    }
}

// the accept loop: every connection is tracked, so a stop drains them for the grace and then drops the rest
async fn serve_bounded(
    listener: TcpListener,
    app: Router,
    stop: CancellationToken,
    grace: Duration,
) -> io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        // reap finished connections, so the set holds only the ones still open
        while connections.try_join_next().is_some() {}
        match select(pin!(listener.accept()), pin!(stop.cancelled())).await {
            Either::Left((accepted, _)) => {
                let (stream, _peer) = accepted?;
                let _serving =
                    connections.spawn(serve_connection(stream, app.clone(), stop.clone()));
            }
            Either::Right(((), _)) => break,
        }
    }
    drain_connections(&mut connections, grace).await;
    Ok(())
}

// graceful for the whole grace, then every connection still open is dropped
async fn drain_connections(connections: &mut JoinSet<io::Result<()>>, grace: Duration) {
    let mut deadline = pin!(time::sleep(grace));
    while !connections.is_empty() {
        match select(deadline.as_mut(), pin!(connections.join_next())).await {
            Either::Left(((), _)) => break,
            Either::Right((_, _)) => {}
        }
    }
    connections.shutdown().await;
}

async fn bind(host: &str, port: u16) -> Result<TcpListener, MetricsServerError> {
    TcpListener::bind((host, port))
        .await
        .map_err(|source| MetricsServerError::Bind {
            host: host.to_owned(),
            port,
            source,
        })
}

// start_metrics_server_process as a task the server owns: the client, the app and the probe are built before the listener starts, so a failure leaves no listener behind, and it stops with the shutdown token and answers before this returns
pub async fn start_metrics_listener(
    settings: &MetricsServerArgs,
    tasks: &TaskTracker,
    shutdown: CancellationToken,
) -> Result<MetricsListener, MetricsServerError> {
    let pid = process::id();
    let client = base_builder()
        .no_proxy()
        .build()
        .map_err(MetricsServerError::Probe)?;
    let folder = settings.multiproc_dir.clone();
    let app = tasks
        .spawn_blocking(move || build_metrics_app(&folder, pid))
        .await??;
    let listener = bind(&settings.host, settings.port).await?;
    let address = listener
        .local_addr()
        .map_err(MetricsServerError::LocalAddress)?;
    let probe = StartupProbe::new(client, &settings.host, address.port(), pid, STARTUP_POLLING);
    let stop = shutdown.child_token();
    let mut serving = tasks.spawn(serve_bounded(listener, app, stop.clone(), SHUTDOWN_GRACE));
    wait_until_serving(&mut serving, &probe, &stop, SHUTDOWN_GRACE).await?;
    Ok(MetricsListener { address, serving })
}

// _exit_when_parent_dies asks for the parent again at every poll, since a reparented process reports a new one
async fn parent_exited<F>(parent: u32, mut current_parent: F, interval: Duration)
where
    F: FnMut() -> u32,
{
    while current_parent() == parent {
        time::sleep(interval).await;
    }
}

// the sidecar keeps serving until the process that started it is gone, then drains like any other stop
async fn serve_until_parent_exits<F>(
    listener: TcpListener,
    app: Router,
    parent: u32,
    current_parent: F,
    grace: Duration,
) -> io::Result<()>
where
    F: FnMut() -> u32 + Send + 'static,
{
    let stop = CancellationToken::new();
    let watcher_stop = stop.clone();
    let exited = stop.clone();
    let mut watcher = JoinSet::new();
    let _watching = watcher.spawn(async move {
        match select(
            pin!(parent_exited(parent, current_parent, PARENT_POLL_INTERVAL)),
            pin!(watcher_stop.cancelled()),
        )
        .await
        {
            Either::Left(((), _)) => exited.cancel(),
            Either::Right(((), _)) => {}
        }
    });
    let outcome = serve_bounded(listener, app, stop.clone(), grace).await;
    stop.cancel();
    watcher.shutdown().await;
    outcome
}

// the sidecar reads its parent before the app is built and the port bound, as LiteLLM reads it before uvicorn builds the app
async fn run_metrics_server_watched<B, F>(
    settings: &MetricsServerArgs,
    build: B,
    mut current_parent: F,
) -> Result<(), MetricsServerError>
where
    B: FnOnce(PathBuf, u32) -> Result<Router, MetricsServerError> + Send + 'static,
    F: FnMut() -> u32 + Send + 'static,
{
    let parent = current_parent();
    let pid = process::id();
    let folder = settings.multiproc_dir.clone();
    let tasks = TaskTracker::new();
    let app = tasks.spawn_blocking(move || build(folder, pid)).await??;
    let listener = bind(&settings.host, settings.port).await?;
    serve_until_parent_exits(listener, app, parent, current_parent, SHUTDOWN_GRACE)
        .await
        .map_err(MetricsServerError::Serve)
}

// run_metrics_server for the sidecar command: it serves until the process that started it is gone
pub async fn run_metrics_server(settings: &MetricsServerArgs) -> Result<(), MetricsServerError> {
    run_metrics_server_watched(
        settings,
        |folder, pid| build_metrics_app(&folder, pid),
        parent_id,
    )
    .await
}

// argparse's default=os.environ.get(...): the environment sets a default, and without one the arg stays required
fn command_for(configured: Option<&Path>) -> Command {
    let command = MetricsServerArgs::augment_args(Command::new(PROGRAM));
    match configured {
        Some(folder) => command.mut_arg("multiproc_dir", |argument| {
            argument
                .default_value_os(folder.as_os_str().to_os_string())
                .required(false)
        }),
        None => command,
    }
}

// the folder flag beats the one the proxy environment carries, as argparse's default= does
pub fn metrics_server_args(
    arguments: Vec<OsString>,
    environment: &BTreeMap<String, String>,
) -> Result<MetricsServerArgs, Error> {
    let configured = environment
        .get(MULTIPROC_DIR)
        .filter(|folder| !folder.is_empty())
        .map(|folder| Path::new(folder.as_str()));
    let command = command_for(configured);
    let matches = command.try_get_matches_from(arguments)?;
    MetricsServerArgs::from_arg_matches(&matches)
}

fn serve(settings: &MetricsServerArgs) -> Result<(), MetricsServerError> {
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(MetricsServerError::Serve)?;
    runtime.block_on(run_metrics_server(settings))
}

fn report(outcome: Result<(), MetricsServerError>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _reported = writeln!(io::stderr(), "{PROGRAM}: {error}");
            ExitCode::FAILURE
        }
    }
}

// the sidecar's entry point, as LiteLLM's __main__: parse the CLI, then serve until the parent process is gone
#[cfg_attr(test, mutants::skip)]
#[must_use]
pub fn main() -> ExitCode {
    match metrics_server_args(env::args_os().collect(), &process_environment()) {
        Ok(settings) => report(serve(&settings)),
        Err(error) => error.exit(),
    }
}
