mod cache;
mod conversation;
mod errors;
mod fixture;
mod input;
mod small;
mod streams;
mod thought;
mod token_steps;
mod trim_steps;
mod validators;

use serde_json::json;

use self::fixture::Session;
use self::input::{Call, Input};
use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| errors::value_error(error.to_string()))?;
        runtime.block_on(async {
            let session = Session::build()?;
            let mut outcomes = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                outcomes.push(dispatch(call, &session).await?);
            }
            Ok(json!(outcomes))
        })
    }
}

async fn dispatch(entry: &Call, session: &Session) -> HelperResult {
    match entry {
        Call::Thought(found) => thought::run(found),
        Call::Conversation(found) => conversation::run(found, session).await,
        Call::TokenSteps(found) => token_steps::run(found, session).await,
        Call::TrimSteps(found) => trim_steps::run(found, session).await,
        Call::Validators(found) => validators::run(found),
        Call::Cache(found) => Ok(cache::run(found, session).await),
        Call::Streams(found) => streams::run(found),
        Call::Small(found) => Ok(small::run(found)),
    }
}
