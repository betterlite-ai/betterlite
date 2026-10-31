#![no_main]

use std::str;

use betterlite_websearch::call_transformation::WebSearchTransformation;
use betterlite_websearch::call_transformation::wire::Response;
use libfuzzer_sys::fuzz_target;

const FORMATS: [&str; 3] = ["anthropic", "openai", "responses"];

fuzz_target!(|bytes: &[u8]| {
    let Ok(body) = str::from_utf8(bytes) else {
        return;
    };
    let Ok(response) = serde_json::from_str::<Response>(body) else {
        return;
    };
    for format in FORMATS {
        let Ok((_, calls)) = WebSearchTransformation::transform_request(&response, false, format)
        else {
            continue;
        };
        if calls.is_empty() {
            continue;
        }
        let results: Vec<String> = calls.iter().map(|_| "result".to_owned()).collect();
        drop(WebSearchTransformation::transform_response(
            &calls, &results, format,
        ));
    }
});
