use betterlite_openai::reasoning_models::DropParams;
use betterlite_prompts::arguments::json::Json;

use super::support;
use crate::responses::capabilities::{
    is_o_series_model, o_series_map_params, o_series_supported_params, supported_params,
};

const GLOBAL_DROP: DropParams = DropParams {
    global: true,
    request: false,
};
const REQUEST_DROP: DropParams = DropParams {
    global: false,
    request: true,
};

fn mapped(params: &str, drop: DropParams) -> Json {
    support::to_json(&o_series_map_params("o3", &support::params(params), drop))
}

pub(super) fn o_series_options() {
    let runtime = support::runtime();
    let mut capabilities = support::fixture();
    // the substring is case-insensitive and does not need the catalog
    assert!(runtime.block_on(is_o_series_model(&mut capabilities, "azure/O_SERIES-prod")));
    // the catalog's reasoning flag answers for any name, not only o1/o3/o4
    assert!(runtime.block_on(is_o_series_model(&mut capabilities, "deepseek-r1")));
    assert!(runtime.block_on(is_o_series_model(&mut capabilities, "o3")));
    assert!(runtime.block_on(is_o_series_model(&mut capabilities, "azure/o3")));
    // an explicit false and an unknown name both answer false
    assert!(!runtime.block_on(is_o_series_model(&mut capabilities, "o3-mini")));
    assert!(!runtime.block_on(is_o_series_model(&mut capabilities, "gpt-4o")));
    o_series_params();
    o_series_mapping();
}

fn o_series_params() {
    let base = supported_params();
    assert!(!base.contains("context_management"));
    assert!(base.contains("temperature"));
    let o_series = o_series_supported_params();
    assert!(o_series.contains("top_p"));
    let removed: Vec<&str> = base.difference(&o_series).copied().collect();
    assert_eq!(removed, ["temperature"]);
}

fn o_series_mapping() {
    let params = r#"{"temperature":0.2,"top_p":0.9,"input":[{"role":"user","content":"x"}]}"#;
    assert_eq!(mapped(params, DropParams::default()), support::json(params));
    // a global or a request-level drop removes temperature and nothing else
    let without = support::json(r#"{"top_p":0.9,"input":[{"role":"user","content":"x"}]}"#);
    assert_eq!(mapped(params, GLOBAL_DROP), without);
    assert_eq!(mapped(params, REQUEST_DROP), without);
    // a null temperature is present, so a drop removes it too
    assert_eq!(
        mapped(r#"{"temperature":null,"top_p":0.9}"#, GLOBAL_DROP),
        support::json(r#"{"top_p":0.9}"#)
    );
    // without temperature the mapper copies every value
    let both = DropParams {
        global: true,
        request: true,
    };
    assert_eq!(
        mapped(r#"{"top_p":0.9}"#, both),
        support::json(r#"{"top_p":0.9}"#)
    );
}
