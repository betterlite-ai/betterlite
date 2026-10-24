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
pub(crate) mod wire;

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
    pub(crate) base_url: String,
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

pub(crate) fn serve(
    listener: TcpListener,
    answers: Vec<Answer>,
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    failure: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut answer_index = 0;
    if should_stop_before_accept(answers.is_empty(), stop.load(Ordering::Relaxed)) {
        return;
    }
    while should_serve(
        stop.load(Ordering::Relaxed),
        answer_index,
        answers.len(),
        deadline_reached(Instant::now(), deadline),
    ) {
        if should_stop_after_answer(stop.load(Ordering::Relaxed), answer_index, answers.len()) {
            break;
        }
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
            Err(error) => {
                if should_retry_accept(&error) {
                    thread::park_timeout(Duration::from_millis(10));
                } else {
                    set_failure(&failure, error.to_string());
                    break;
                }
            }
        }
    }
    if should_report_timeout(
        stop.load(Ordering::Relaxed),
        answer_index,
        answers.len(),
        deadline_reached(Instant::now(), deadline),
    ) {
        set_failure(
            &failure,
            "timed out waiting for provider requests".to_owned(),
        );
    }
}

pub(crate) fn should_serve(
    stopped: bool,
    answer_index: usize,
    answer_count: usize,
    deadline_reached: bool,
) -> bool {
    let should_serve = !stopped && answer_index < answer_count && !deadline_reached;
    should_serve
}

pub(crate) fn should_stop_before_accept(answers_empty: bool, stopped: bool) -> bool {
    let should_stop = answers_empty || stopped;
    should_stop
}

pub(crate) fn should_stop_after_answer(
    stopped: bool,
    answer_index: usize,
    answer_count: usize,
) -> bool {
    let should_stop = stopped || answer_index >= answer_count;
    should_stop
}

pub(crate) fn should_report_timeout(
    stopped: bool,
    answer_index: usize,
    answer_count: usize,
    deadline_reached: bool,
) -> bool {
    let has_answers = answer_index < answer_count;
    let should_report = match (stopped, has_answers, deadline_reached) {
        (false, true, true) => true,
        (true, _, _) | (false, false, _) | (false, true, false) => false,
    };
    should_report
}

pub(crate) fn deadline_reached(now: Instant, deadline: Instant) -> bool {
    let reached = now >= deadline;
    reached
}

pub(crate) fn should_retry_accept(error: &Error) -> bool {
    let would_block = error.kind() == std::io::ErrorKind::WouldBlock;
    would_block
}

pub(crate) fn set_failure(failure: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut slot) = failure.lock() {
        *slot = Some(message);
    }
}
