use std::io;
use std::net::SocketAddr;
use std::os::unix::process::parent_id;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::map_response_with_state;
use axum::response::{IntoResponse as _, Response};
use axum::routing::any;
use axum::{Json, Router};
use betterlite_prometheus::multiprocess::MultiprocessError;
use betterlite_prometheus::multiprocess::collector::MultiProcessCollector;
use betterlite_prometheus::scrape_endpoint::ScrapeSource;
use betterlite_prometheus::scrape_endpoint::app::make_metrics_app;
use clap::Args;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::task::{JoinError, JoinHandle};
use tokio::time;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

mod tests;

pub const METRICS_PATH: &str = "/metrics";
pub const HEALTH_PATH: &str = "/health";
pub const PID_HEADER: &str = "x-litellm-metrics-pid";
pub const PARENT_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const STARTUP_POLLING: StartupPolling = StartupPolling {
    deadline: Duration::from_secs(30),
    interval: Duration::from_millis(100),
    probe_timeout: Duration::from_secs(1),
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupPolling {
    pub deadline: Duration,
    pub interval: Duration,
    pub probe_timeout: Duration,
}

// what the startup wait asks of the metrics port: a 200 that carries this pid
#[derive(Clone, Debug)]
pub struct StartupProbe<'client> {
    pub client: &'client reqwest::Client,
    pub url: String,
    pub pid: u32,
    pub polling: StartupPolling,
}

#[derive(Debug)]
pub struct MetricsListener {
    pub address: SocketAddr,
    pub serving: JoinHandle<io::Result<()>>,
}

// LiteLLM's prometheus_metrics_server CLI: --port has no default, the folder comes from the flag or the environment
#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct MetricsServerArgs {
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    #[arg(long)]
    pub port: u16,
    #[arg(long = "multiproc_dir", env = "PROMETHEUS_MULTIPROC_DIR")]
    pub multiproc_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsServerError {
    #[error("the Prometheus multiprocess folder {} cannot be served", path.display())]
    Folder {
        path: PathBuf,
        #[source]
        source: MultiprocessError,
    },
    #[error(
        "Prometheus metrics server cannot listen on {host}:{port}; is the port already in use?"
    )]
    Bind {
        host: String,
        port: u16,
        #[source]
        source: io::Error,
    },
    #[error("cannot read the address the Prometheus metrics server listens on")]
    LocalAddress(#[source] io::Error),
    #[error("Prometheus metrics server stopped before serving {url}")]
    Stopped {
        url: String,
        #[source]
        source: Option<io::Error>,
    },
    #[error("Prometheus metrics server did not answer {url} within {seconds}s")]
    Timeout { url: String, seconds: u64 },
    #[error("Prometheus metrics server failed")]
    Serve(#[source] io::Error),
}

#[derive(Serialize)]
struct Health<'folder> {
    status: &'static str,
    multiproc_dir: &'folder Path,
}

#[derive(Serialize)]
struct Detail {
    detail: &'static str,
}

// build_metrics_app: the scrape app under /metrics with the pid on every answer, and FastAPI's /health and 404 beside it
pub fn build_metrics_app(multiproc_dir: &Path, pid: u32) -> Result<Router, MetricsServerError> {
    let collector =
        MultiProcessCollector::new(multiproc_dir).map_err(|source| MetricsServerError::Folder {
            path: multiproc_dir.to_path_buf(),
            source,
        })?;
    let scrapes = make_metrics_app(ScrapeSource::Multiprocess(collector))
        .layer(map_response_with_state(HeaderValue::from(pid), add_pid));
    let app = Router::new()
        .route(HEALTH_PATH, any(health))
        .nest_service(METRICS_PATH, scrapes)
        .fallback(|| async {
            let body = Json(Detail {
                detail: "Not Found",
            });
            (StatusCode::NOT_FOUND, body).into_response()
        })
        .with_state(Arc::<Path>::from(multiproc_dir));
    Ok(app)
}

// _add_pid_header appends to what the scrape app already set, so a client can tell a restarted listener
async fn add_pid(State(pid): State<HeaderValue>, mut response: Response) -> Response {
    let _appended = response
        .headers_mut()
        .append(HeaderName::from_static(PID_HEADER), pid);
    response
}

// FastAPI's @app.get answers only GET: HEAD and the rest are a 405 that names GET
async fn health(State(multiproc_dir): State<Arc<Path>>, method: Method) -> Response {
    if method != Method::GET {
        let body = Json(Detail {
            detail: "Method Not Allowed",
        });
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET")],
            body,
        )
            .into_response();
    }
    let body = Health {
        status: "healthy",
        multiproc_dir: &multiproc_dir,
    };
    Json(body).into_response()
}

// the probe can't dial a wildcard address, and an IPv6 host goes in brackets
#[must_use]
pub fn metrics_url(host: &str, port: u16) -> String {
    let probe_host = match host {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    if probe_host.contains(':') {
        format!("http://[{probe_host}]:{port}{METRICS_PATH}")
    } else {
        format!("http://{probe_host}:{port}{METRICS_PATH}")
    }
}

// _answered_by: only our own listener counts, not whatever else holds the port, and a failed request is a no
pub async fn answered_by(probe: &StartupProbe<'_>) -> bool {
    let request = probe
        .client
        .get(probe.url.as_str())
        .timeout(probe.polling.probe_timeout);
    let Ok(response) = request.send().await else {
        return false;
    };
    let pid = probe.pid.to_string();
    response.status() == StatusCode::OK
        && response
            .headers()
            .get(PID_HEADER)
            .is_some_and(|value| value.as_bytes() == pid.as_bytes())
}

// _wait_until_serving: a listener that stopped fails at once, one that never answers is stopped at the deadline
pub async fn wait_until_serving(
    serving: &mut JoinHandle<io::Result<()>>,
    probe: &StartupProbe<'_>,
) -> Result<(), MetricsServerError> {
    let polled = time::timeout(probe.polling.deadline, async {
        loop {
            if serving.is_finished() {
                return Err(stopped(&probe.url, (&mut *serving).await));
            }
            if answered_by(probe).await {
                return Ok(());
            }
            time::sleep(probe.polling.interval).await;
        }
    })
    .await;
    polled.unwrap_or_else(|_elapsed| {
        serving.abort();
        Err(MetricsServerError::Timeout {
            url: probe.url.clone(),
            seconds: probe.polling.deadline.as_secs(),
        })
    })
}

fn stopped(url: &str, outcome: Result<io::Result<()>, JoinError>) -> MetricsServerError {
    let source = match outcome {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(join) => Some(io::Error::from(join)),
    };
    MetricsServerError::Stopped {
        url: url.to_owned(),
        source,
    }
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
    client: &reqwest::Client,
) -> Result<MetricsListener, MetricsServerError> {
    let pid = process::id();
    let app = build_metrics_app(&settings.multiproc_dir, pid)?;
    let listener = bind(&settings.host, settings.port).await?;
    let address = listener
        .local_addr()
        .map_err(MetricsServerError::LocalAddress)?;
    let mut serving = tasks.spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    let probe = StartupProbe {
        client,
        url: metrics_url(&settings.host, address.port()),
        pid,
        polling: STARTUP_POLLING,
    };
    wait_until_serving(&mut serving, &probe).await?;
    Ok(MetricsListener { address, serving })
}

// run_metrics_server for the sidecar command: it serves until the process that started it is gone
pub async fn run_metrics_server(settings: &MetricsServerArgs) -> Result<(), MetricsServerError> {
    let parent = parent_id();
    let app = build_metrics_app(&settings.multiproc_dir, process::id())?;
    let listener = bind(&settings.host, settings.port).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(parent_exited(parent, parent_id, PARENT_POLL_INTERVAL))
        .await
        .map_err(MetricsServerError::Serve)
}

// _exit_when_parent_dies asks for the parent again at every poll, since a reparented process reports a new one
pub async fn parent_exited<F>(parent: u32, mut current_parent: F, interval: Duration)
where
    F: FnMut() -> u32,
{
    while current_parent() == parent {
        time::sleep(interval).await;
    }
}
