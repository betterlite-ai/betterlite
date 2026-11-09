use std::sync::{Mutex, PoisonError};

use betterlite_models::ollama_model_metadata::{OllamaTransport, ShowRequest, ShowResponse};
use secrecy::ExposeSecret as _;

use crate::helpers::HelperError;

#[derive(Clone, Debug)]
pub(super) enum FixtureOutcome {
    Reply { status: u16, body: Vec<u8> },
    Failure,
}

#[derive(Clone, Debug)]
pub(super) struct Recorded {
    url: String,
    authorization: Option<String>,
    name: String,
}

pub(super) struct FixtureTransport {
    outcome: FixtureOutcome,
    recorded: Mutex<Vec<Recorded>>,
}

#[derive(Debug, thiserror::Error)]
#[error("fixture transport failure")]
pub(super) struct FixtureFailure;

impl FixtureTransport {
    pub(super) fn new(outcome: FixtureOutcome) -> Self {
        Self {
            outcome,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn only_request(&self) -> Result<Recorded, Box<HelperError>> {
        let mut recorded = self.recorded();
        if recorded.len() != 1 {
            return Err(helper_error(
                "Fixture",
                format!("expected one request, recorded {}", recorded.len()),
            ));
        }
        Ok(recorded.remove(0))
    }
}

impl OllamaTransport for FixtureTransport {
    type Error = FixtureFailure;

    fn post_show(&self, request: &ShowRequest) -> Result<ShowResponse, Self::Error> {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| *name == "Authorization")
            .map(|(_, value)| value.expose_secret().to_owned());
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.clone(),
                authorization,
                name: request.body.name.to_owned(),
            });
        match &self.outcome {
            FixtureOutcome::Reply { status, body } => Ok(ShowResponse {
                status: *status,
                body: body.clone(),
            }),
            FixtureOutcome::Failure => Err(FixtureFailure),
        }
    }
}

pub(super) fn success_reply() -> FixtureTransport {
    FixtureTransport::new(FixtureOutcome::Reply {
        status: 200,
        body: b"{}".to_vec(),
    })
}
