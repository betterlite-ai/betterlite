use std::collections::BTreeMap;
use std::mem::take;
use std::time::Instant;

use secrecy::SecretString;

use super::super::error::ScenarioError;
use super::super::model::{
    CapturedValue, HttpScenario, POLLING_KEY_PREFIX, POLLING_TTL_SECONDS, Placeholders, Step,
    response_document, scalar_text, substitute_value,
};
use super::{
    Clock, Peer, PeerFactory, PeerFixture, PeerRole, PeerStart, PeerTranscript, PhysicalNames,
    StepCapture, child_environment, render_config,
};
use crate::case::Request;
use crate::http::Response;

pub(super) struct PeerRun<'input> {
    pub(super) role: PeerRole,
    pub(super) scenario: &'input HttpScenario,
    pub(super) clock: &'input dyn Clock,
    pub(super) deadline: Instant,
    pub(super) upstream: &'input str,
    pub(super) fixture: PeerFixture<'input>,
    pub(super) peer: Option<Box<dyn Peer>>,
    pub(super) captures: BTreeMap<String, CapturedValue>,
}

pub(super) fn check_deadline(clock: &dyn Clock, deadline: Instant) -> Result<(), ScenarioError> {
    if clock.now() >= deadline {
        return Err(ScenarioError::Deadline);
    }
    Ok(())
}

pub(super) fn run_peer(
    mut state: PeerRun<'_>,
    factory: &mut dyn PeerFactory,
) -> Result<PeerTranscript, ScenarioError> {
    let operation = state.execute(factory);
    let cleanup = state.close();
    match (operation, cleanup) {
        (Ok(transcript), Ok(())) => Ok(transcript),
        (Ok(_), Err(error)) | (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(ScenarioError::OperationCleanup {
            operation: Box::new(error),
            cleanup: Box::new(cleanup_error),
        }),
    }
}

impl PeerRun<'_> {
    fn execute(&mut self, factory: &mut dyn PeerFactory) -> Result<PeerTranscript, ScenarioError> {
        check_deadline(self.clock, self.deadline)?;
        let need_database =
            self.scenario.setup.row_count() > 0 || self.scenario.config.contains("{{postgres}}");
        let names = self.fixture.repository.provision(need_database)?;
        check_deadline(self.clock, self.deadline)?;
        let journal = self.fixture.repository.insert(&self.scenario.setup)?;
        self.write_polling(&names)?;
        check_deadline(self.clock, self.deadline)?;
        let config = render_config(self.scenario, &names, self.upstream);
        let env = child_environment(&self.scenario.env);
        let peer = factory.start(PeerStart {
            role: self.role,
            config: &config,
            env: &env,
        })?;
        self.peer = Some(peer);
        let mut steps = self.run_steps()?;
        check_deadline(self.clock, self.deadline)?;
        self.apply_splits(&mut steps)?;
        check_deadline(self.clock, self.deadline)?;
        let log = self.stop_and_log()?;
        Ok(PeerTranscript {
            names,
            journal,
            steps,
            captures: take(&mut self.captures),
            log,
        })
    }

    fn apply_splits(&mut self, steps: &mut [StepCapture]) -> Result<(), ScenarioError> {
        let counts: Vec<usize> = self
            .scenario
            .steps
            .iter()
            .map(|step| step.upstream.len())
            .collect();
        let split_list = self
            .peer
            .as_deref_mut()
            .ok_or(ScenarioError::Peer)?
            .provider_splits(&counts)?;
        if let Some(splits) = split_list {
            for (capture, split) in steps.iter_mut().zip(splits) {
                capture.provider = split.requests;
                capture.provider_total = split.total;
            }
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), ScenarioError> {
        let stopped = self.peer.take().map_or(Ok(()), |mut peer| peer.stop());
        let closed = self.fixture.repository.close();
        combine_cleanup(stopped, closed)
    }

    fn write_polling(&mut self, names: &PhysicalNames) -> Result<(), ScenarioError> {
        for entry in &self.scenario.setup.polling {
            let key = format!("{POLLING_KEY_PREFIX}{}", entry.name);
            self.fixture.polling.write(
                &names.redis_prefix,
                &key,
                &entry.value,
                POLLING_TTL_SECONDS,
            )?;
        }
        Ok(())
    }

    fn run_steps(&mut self) -> Result<Vec<StepCapture>, ScenarioError> {
        let scenario = self.scenario;
        let mut results = Vec::with_capacity(scenario.steps.len());
        let mut previous = 0_usize;
        for (index, step) in scenario.steps.iter().enumerate() {
            check_deadline(self.clock, self.deadline)?;
            let request = self.build_request(index, step)?;
            let response = self
                .peer
                .as_deref_mut()
                .ok_or(ScenarioError::Peer)?
                .send(&request)?;
            check_deadline(self.clock, self.deadline)?;
            let cumulative = self
                .peer
                .as_deref_mut()
                .ok_or(ScenarioError::Peer)?
                .provider_requests()?;
            let provider = cumulative
                .get(previous..)
                .ok_or(ScenarioError::ProviderTraceShrank)?
                .to_vec();
            previous = cumulative.len();
            self.apply_captures(index, step, &response)?;
            results.push(StepCapture {
                response,
                provider,
                provider_total: cumulative.len(),
            });
        }
        Ok(results)
    }

    fn build_request(&self, index: usize, step: &Step) -> Result<Request, ScenarioError> {
        let placeholders = Placeholders {
            index,
            captures: &self.captures,
        };
        let path = placeholders.resolve(&step.request.path)?;
        let mut headers = BTreeMap::new();
        for (name, value) in &step.request.headers {
            headers.insert(name.clone(), placeholders.resolve(value)?);
        }
        let body = substitute_value(&step.request.body, &placeholders)?;
        Ok(Request {
            method: step.request.method.clone(),
            path,
            headers,
            body,
            body_kind: step.request.body_kind,
        })
    }

    fn apply_captures(
        &mut self,
        index: usize,
        step: &Step,
        response: &Response,
    ) -> Result<(), ScenarioError> {
        let document = response_document(response)?;
        for capture in &step.capture {
            let value = document.pointer(&capture.pointer).ok_or_else(|| {
                ScenarioError::MissingPointer {
                    index,
                    name: capture.name.clone(),
                    pointer: capture.pointer.clone(),
                }
            })?;
            let text = scalar_text(value).ok_or_else(|| ScenarioError::CaptureNotScalar {
                index,
                name: capture.name.clone(),
            })?;
            self.captures.insert(
                capture.name.clone(),
                CapturedValue {
                    text: SecretString::from(text),
                    secret: capture.secret,
                },
            );
        }
        Ok(())
    }

    fn stop_and_log(&mut self) -> Result<String, ScenarioError> {
        let Some(mut peer) = self.peer.take() else {
            return Ok(String::new());
        };
        peer.stop()?;
        peer.log()
    }
}

fn combine_cleanup(
    first: Result<(), ScenarioError>,
    second: Result<(), ScenarioError>,
) -> Result<(), ScenarioError> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(operation), Err(cleanup)) => Err(ScenarioError::OperationCleanup {
            operation: Box::new(operation),
            cleanup: Box::new(cleanup),
        }),
    }
}
