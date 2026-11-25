use std::io;
use std::path::{Path, PathBuf};
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
use reqwest::Client;
use serde::Serialize;
use tokio::task::{JoinError, JoinHandle};
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::proxy::base_builder;

mod tests;

pub const METRICS_PATH: &str = "/metrics";
pub const HEALTH_PATH: &str = "/health";
pub const PID_HEADER: &str = "x-litellm-metrics-pid";
pub const STARTUP_POLLING: StartupPolling = StartupPolling {
    deadline: Duration::from_secs(30),
    interval: Duration::from_millis(100),
    probe_timeout: Duration::from_secs(1),
};

const CLEANUP_POLL: Duration = Duration::from_millis(10);
const CLEANUP_MARGIN: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupPolling {
    pub deadline: Duration,
    pub interval: Duration,
    pub probe_timeout: Duration,
}

// what the startup wait asks of the metrics port: a 200 that carries this pid
#[derive(Clone, Debug)]
pub struct StartupProbe {
    client: Client,
    url: String,
    pid: u32,
    polling: StartupPolling,
}

impl StartupProbe {
    pub fn new(
        host: &str,
        port: u16,
        pid: u32,
        polling: StartupPolling,
    ) -> Result<Self, MetricsServerError> {
        let client = base_builder()
            .no_proxy()
            .build()
            .map_err(MetricsServerError::Probe)?;
        Ok(Self {
            client,
            url: metrics_url(host, port),
            pid,
            polling,
        })
    }
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
    #[error("the client the Prometheus metrics server probe asks through cannot be built")]
    Probe(#[source] reqwest::Error),
    #[error("Prometheus metrics server failed")]
    Serve(#[source] io::Error),
    #[error("a Prometheus metrics server task failed")]
    Task(#[from] JoinError),
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
pub async fn answered_by(probe: &StartupProbe) -> bool {
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
    probe: &StartupProbe,
    stop: &CancellationToken,
    grace: Duration,
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
    if let Ok(served) = polled {
        return served;
    }
    stop.cancel();
    let deadline = time::Instant::now() + grace.saturating_add(CLEANUP_MARGIN);
    while !serving.is_finished() && time::Instant::now() < deadline {
        time::sleep(CLEANUP_POLL).await;
    }
    if !serving.is_finished() {
        serving.abort();
    }
    Err(MetricsServerError::Timeout {
        url: probe.url.clone(),
        seconds: probe.polling.deadline.as_secs(),
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
