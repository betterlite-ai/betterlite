use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use secrecy::ExposeSecret as _;

use crate::hiddenlayer::hiddenlayer_authentication::token::{JwtError, JwtTransport, TokenRequest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) api_id: String,
    pub(crate) api_key: String,
}

// A token transport that answers from a script and remembers every request it was asked for
pub(crate) struct ScriptedTokens {
    queue: Mutex<VecDeque<String>>,
    fetches: Mutex<Vec<Recorded>>,
}

impl ScriptedTokens {
    pub(crate) fn new(tokens: &[&str]) -> Self {
        Self {
            queue: Mutex::new(tokens.iter().map(|token| (*token).to_owned()).collect()),
            fetches: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn fetches(&self) -> Vec<Recorded> {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl JwtTransport for ScriptedTokens {
    fn fetch_token(&self, request: &TokenRequest) -> Result<String, JwtError> {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.clone(),
                api_id: request.api_id.clone(),
                api_key: request.api_key.expose_secret().to_owned(),
            });
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| JwtError::InvalidResponse {
                body: "no token scripted".to_owned(),
            })
    }
}
