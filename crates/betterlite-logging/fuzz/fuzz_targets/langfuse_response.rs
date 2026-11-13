#![no_main]

use betterlite_logging::langfuse_rest::records::{Projects, Prompt};
use betterlite_logging::langfuse_rest::transport::HttpResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let response = HttpResponse {
        status: 200,
        body: body.to_vec(),
    };
    let _decoded = response.json();
    let _prompt = serde_json::from_slice::<Prompt>(body);
    let _projects = serde_json::from_slice::<Projects>(body);
});
