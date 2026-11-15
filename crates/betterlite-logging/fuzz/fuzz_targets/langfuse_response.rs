#![no_main]

use betterlite_logging::langfuse_prompt_management::prompt::compile_prompt;
use betterlite_logging::langfuse_rest::records::{Projects, Prompt};
use betterlite_logging::langfuse_rest::transport::HttpResponse;
use betterlite_models::prompt_contracts::PromptParams;
use libfuzzer_sys::fuzz_target;

const VARIABLES: &str =
    r#"{"name":"world","role":"assistant","history":[{"role":"user","content":"hello {{name}}"}]}"#;

fuzz_target!(|body: &[u8]| {
    let response = HttpResponse {
        status: 200,
        body: body.to_vec(),
    };
    let _decoded = response.json();
    let Ok(prompt) = serde_json::from_slice::<Prompt>(body) else {
        return;
    };
    let _projects = serde_json::from_slice::<Projects>(body);
    let Ok(variables) = serde_json::from_str::<PromptParams>(VARIABLES) else {
        return;
    };
    let _compiled = compile_prompt(&prompt, Some(&variables));
});
