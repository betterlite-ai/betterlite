#![no_main]

use betterlite_guardrails::lakera::response::LakeraModerationResponse;
use betterlite_guardrails::lasso::MaskedMessages;
use betterlite_guardrails::lasso_masking::wire::LassoMessage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _moderation = serde_json::from_slice::<LakeraModerationResponse>(bytes);
    let _classifix = serde_json::from_slice::<MaskedMessages>(bytes);
    let _messages = serde_json::from_slice::<Vec<LassoMessage>>(bytes);
});
