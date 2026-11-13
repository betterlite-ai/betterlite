#![no_main]

use std::convert::Infallible;

use betterlite_models::generic_prompt_api::fetch::read_response;
use betterlite_models::generic_prompt_api::request::PromptApiResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let answer = PromptApiResponse {
        status: 200,
        body: body.to_vec(),
    };
    let _outcome = read_response::<Infallible>(Some("welcome"), Ok(answer));
});
