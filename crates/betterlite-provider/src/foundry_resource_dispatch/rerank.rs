use std::collections::BTreeMap;

use betterlite_foundry::rerank::error::FoundryRerankError;
use betterlite_foundry::rerank::reply::{FoundryRerankReply, transform_reply};
use betterlite_foundry::rerank::request::{FoundryRerankRequest, transform_request};
use betterlite_foundry::rerank::url::complete_url;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank::RerankParams;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Rerank(#[from] FoundryRerankError),
}

// B556's three rerank steps: the Foundry URL, the Cohere v1 body and the reply with its ranking
pub struct Dispatch<'call> {
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Url {
        api_base: Option<&'call str>,
    },
    Request(&'call FoundryRerankRequest),
    Reply {
        status: u16,
        body: &'call [u8],
        limits: WireDecodeLimits,
        hidden_params: BTreeMap<String, Json>,
    },
}

pub enum Outcome {
    Url(String),
    Request(RerankParams),
    Reply(FoundryRerankReply),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, Error> {
        Ok(match self.call {
            Call::Url { api_base } => Outcome::Url(complete_url(api_base)?),
            Call::Request(request) => Outcome::Request(transform_request(request)?),
            Call::Reply {
                status,
                body,
                limits,
                hidden_params,
            } => Outcome::Reply(transform_reply(status, body, limits, hidden_params)?),
        })
    }
}
