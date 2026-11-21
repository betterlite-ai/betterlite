use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::vector_search::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde_json::json;

use super::support::{OTHER, failure, runtime};

// B558's index path split is what the search route maps first, so the case reads both lists
pub(super) fn run() -> HelperResult {
    let runtime = runtime()?;
    let operation = FoundryResourceOperation::VectorSearch(Box::new(Dispatch {
        call: Call::Endpoints,
    }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::VectorSearch(Outcome::Endpoints(found))) => {
            Ok(json!({"read": found.read, "write": found.write}))
        }
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
}
