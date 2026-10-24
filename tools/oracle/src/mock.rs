use std::collections::BTreeMap;
use std::io::Error;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::case::UpstreamAnswer;

#[path = "mock_wire.rs"]
mod wire;

use wire::{Answer, handle_connection, to_answer};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

#[derive(Debug, Error)]
pub(crate) enum MockError {
    #[error("cannot bind mock provider: {0}")]
    Bind(#[source] Error),
    #[error("mock provider failed: {0}")]
    Server(String),
    #[error("mock provider thread panicked")]
    Join,
}

pub(crate) struct MockProvider {
    base_url: String,
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    failure: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl MockProvider {
    pub(crate) fn start(answers: &[UpstreamAnswer]) -> Result<Self, MockError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(MockError::Bind)?;
        listener.set_nonblocking(true).map_err(MockError::Bind)?;
        let address = listener.local_addr().map_err(MockError::Bind)?;
        let prepared_answers = answers.iter().map(to_answer).collect::<Vec<_>>();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let failure = Arc::new(Mutex::new(None));
        let thread_requests = Arc::clone(&requests);
        let thread_failure = Arc::clone(&failure);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("oracle-mock".to_owned())
            .spawn(move || {
                serve(
                    listener,
                    prepared_answers,
                    thread_requests,
                    thread_failure,
                    thread_stop,
                );
            })
            .map_err(MockError::Bind)?;
        Ok(Self {
            base_url: format!("http://{address}"),
            requests,
            failure,
            stop,
            thread,
        })
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn finish(self) -> Result<Vec<ProviderRequest>, MockError> {
        self.stop.store(true, Ordering::Relaxed);
        if self.thread.join().is_err() {
            return Err(MockError::Join);
        }
        let failure = self
            .failure
            .lock()
            .map_err(|error| MockError::Server(format!("failure lock poisoned: {error}")))?
            .take();
        if let Some(message) = failure {
            return Err(MockError::Server(message));
        }
        let requests = self
            .requests
            .lock()
            .map_err(|error| MockError::Server(format!("request lock poisoned: {error}")))?
            .clone();
        Ok(requests)
    }
}

fn serve(
    listener: TcpListener,
    answers: Vec<Answer>,
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    failure: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut answer_index = 0;
    while !stop.load(Ordering::Relaxed) && answer_index < answers.len() && Instant::now() < deadline
    {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let Some(answer) = answers.get(answer_index) else {
                    set_failure(&failure, "provider answer sequence exhausted".to_owned());
                    break;
                };
                match handle_connection(&mut stream, answer) {
                    Ok(Some(request)) => match requests.lock() {
                        Ok(mut stored) => {
                            stored.push(request);
                            answer_index = answer_index.saturating_add(1);
                        }
                        Err(error) => {
                            set_failure(&failure, format!("request lock poisoned: {error}"));
                            break;
                        }
                    },
                    Ok(None) => {}
                    Err(error) => {
                        set_failure(&failure, error.to_string());
                        break;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::park_timeout(Duration::from_millis(10));
            }
            Err(error) => {
                set_failure(&failure, error.to_string());
                break;
            }
        }
    }
    if !stop.load(Ordering::Relaxed) && answer_index < answers.len() && Instant::now() >= deadline {
        set_failure(
            &failure,
            "timed out waiting for provider requests".to_owned(),
        );
    }
}

fn set_failure(failure: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut slot) = failure.lock() {
        *slot = Some(message);
    }
}
