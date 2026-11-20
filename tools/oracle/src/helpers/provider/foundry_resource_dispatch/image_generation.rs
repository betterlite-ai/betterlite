use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::image_generation::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{OTHER, Recorder, failure, runtime};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    model: String,
}

// B553's config selection, the branch every later Flux, MAI, DALL-E and GPT step runs under
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let mut logging = Recorder;
    let operation = FoundryResourceOperation::ImageGeneration(Box::new(Dispatch {
        logging: &mut logging,
        call: Call::Config {
            model: &input.model,
        },
    }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ImageGeneration(outcome)) => Ok(answer(outcome)),
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
}

fn answer(outcome: Outcome) -> Value {
    match outcome {
        Outcome::Config(class) => json!(class),
        Outcome::Url(url) => json!(url),
        Outcome::FluxParams(params) => json!(params),
        Outcome::MaiParams(params) => json!(params),
        Outcome::Reply(response) => json!({ "size": response.size }),
    }
}
