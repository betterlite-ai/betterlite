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
use clap::{Args, Command, Error, FromArgMatches as _};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnectionBuilder;
use hyper_util::service::TowerToHyperService;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

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
    let builder = ConnectionBuilder::new(TokioExecutor::new());
    let service = TowerToHyperService::new(app);
    let mut connection = pin!(builder.serve_connection(TokioIo::new(stream), service));
    tokio::select! {
        served = connection.as_mut() => served.map_err(io::Error::other),
        () = stop.cancelled() => {
            connection.as_mut().graceful_shutdown();
            connection.as_mut().await.map_err(io::Error::other)
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
        while let Some(joined) = connections.try_join_next() {
            let _done = joined;
        }
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _peer) = accepted?;
                let _serving = connections.spawn(serve_connection(stream, app.clone(), stop.clone()));
            }
            () = stop.cancelled() => break,
        }
    }
    drain_connections(&mut connections, grace).await;
    Ok(())
}

// graceful for the whole grace, then every connection still open is dropped
async fn drain_connections(connections: &mut JoinSet<io::Result<()>>, grace: Duration) {
    let mut deadline = pin!(time::sleep(grace));
    while !connections.is_empty() {
        tokio::select! {
            () = &mut deadline => break,
            joined = connections.join_next() => {
                let _done = joined;
            }
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

// start_metrics_server_process as a task the server owns: it stops with the shutdown token and answers before this returns
pub async fn start_metrics_listener(
    settings: &MetricsServerArgs,
    tasks: &TaskTracker,
    shutdown: CancellationToken,
) -> Result<MetricsListener, MetricsServerError> {
    let pid = process::id();
    let folder = settings.multiproc_dir.clone();
    let app = tasks
        .spawn_blocking(move || build_metrics_app(&folder, pid))
        .await??;
    let listener = bind(&settings.host, settings.port).await?;
    let address = listener
        .local_addr()
        .map_err(MetricsServerError::LocalAddress)?;
    let stop = shutdown.child_token();
    let mut serving = tasks.spawn(serve_bounded(listener, app, stop.clone(), SHUTDOWN_GRACE));
    let probe = StartupProbe::new(&settings.host, address.port(), pid, STARTUP_POLLING)?;
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
        tokio::select! {
            () = parent_exited(parent, current_parent, PARENT_POLL_INTERVAL) => exited.cancel(),
            () = watcher_stop.cancelled() => (),
        }
    });
    let outcome = serve_bounded(listener, app, stop.clone(), grace).await;
    stop.cancel();
    watcher.shutdown().await;
    outcome
}

// run_metrics_server for the sidecar command: it serves until the process that started it is gone
pub async fn run_metrics_server(settings: &MetricsServerArgs) -> Result<(), MetricsServerError> {
    let pid = process::id();
    let folder = settings.multiproc_dir.clone();
    let tasks = TaskTracker::new();
    let app = tasks
        .spawn_blocking(move || build_metrics_app(&folder, pid))
        .await??;
    let listener = bind(&settings.host, settings.port).await?;
    serve_until_parent_exits(listener, app, parent_id(), parent_id, SHUTDOWN_GRACE)
        .await
        .map_err(MetricsServerError::Serve)
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
    let mut command = MetricsServerArgs::augment_args(Command::new(PROGRAM));
    if let Some(folder) = configured {
        command = command.mut_arg("multiproc_dir", |argument| {
            argument
                .default_value_os(folder.as_os_str().to_os_string())
                .required(false)
        });
    }
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
pub fn main() -> ExitCode {
    match metrics_server_args(env::args_os().collect(), &process_environment()) {
        Ok(settings) => report(serve(&settings)),
        Err(error) => error.exit(),
    }
}

#[cfg(test)]
mod metrics_server {
    include!("metrics_listener/tests.rs");
}
