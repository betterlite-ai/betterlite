#![cfg(test)]

use super::Params;

mod extra_body;
mod filters;
mod function_fallback;
mod pre_process;
mod reasoning;

fn params(text: &str) -> Params {
    serde_json::from_str(text).unwrap()
}
