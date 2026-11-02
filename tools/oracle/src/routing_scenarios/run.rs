use std::iter::repeat_n;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use reqwest::blocking::{Client, ClientBuilder};
use reqwest::redirect::Policy;

use super::{HttpStep, JoinPoint, Scenario, ScenarioStep};
use crate::OracleError;
use crate::http::{self, Response};

type ConcurrentResult = (usize, Result<Response, OracleError>);

pub(crate) fn run_scenario(
    client: &Client,
    base: &str,
    scenario: &Scenario,
) -> Result<Vec<Response>, OracleError> {
    let mut responses = Vec::new();
    let mut pending: Option<&[HttpStep]> = None;
    for step in &scenario.steps {
        match step {
            ScenarioStep::Http(http_step) => {
                responses.push(http::send(client, base, &http_step.request)?);
            }
            ScenarioStep::Concurrent(concurrent) => pending = Some(&concurrent.requests),
            ScenarioStep::Join(join) => {
                let requests = pending.take().ok_or(OracleError::ScenarioJoinMissing)?;
                responses.extend(run_concurrent(base, requests, join)?);
            }
        }
    }
    Ok(responses)
}

// Bounds each request to the join deadline, so a slow reply still lets thread::scope's join land on time.
#[expect(
    clippy::disallowed_methods,
    reason = "A concurrent step times its own client to the join deadline instead of the shared 30s default."
)]
fn deadline_client(deadline: Duration) -> Result<Client, OracleError> {
    ClientBuilder::from(reqwest::Client::builder())
        .no_proxy()
        .redirect(Policy::none())
        .timeout(deadline)
        .build()
        .map_err(OracleError::HttpRequest)
}

fn run_concurrent(
    base: &str,
    requests: &[HttpStep],
    join: &JoinPoint,
) -> Result<Vec<Response>, OracleError> {
    let deadline = Duration::from_millis(join.deadline_ms);
    let client = deadline_client(deadline)?;
    let (sender, receiver) = mpsc::sync_channel::<ConcurrentResult>(requests.len());
    thread::scope(|scope| {
        for (index, step) in requests.iter().enumerate() {
            let tx = sender.clone();
            let request_client = &client;
            scope.spawn(move || {
                let result = http::send(request_client, base, &step.request);
                let _sent = tx.send((index, result));
            });
        }
        drop(sender);
        collect(&receiver, requests.len(), deadline)
    })
}

fn collect(
    receiver: &mpsc::Receiver<ConcurrentResult>,
    count: usize,
    deadline: Duration,
) -> Result<Vec<Response>, OracleError> {
    let mut collected: Vec<Option<Response>> = repeat_n(None, count).collect();
    for _received in 0..count {
        let (index, result) = receiver
            .recv_timeout(deadline)
            .map_err(OracleError::ScenarioJoinFailed)?;
        let slot = collected
            .get_mut(index)
            .ok_or(OracleError::MissingCapture(index))?;
        *slot = Some(result?);
    }
    collected
        .into_iter()
        .enumerate()
        .map(|(index, response)| response.ok_or(OracleError::MissingCapture(index)))
        .collect()
}
