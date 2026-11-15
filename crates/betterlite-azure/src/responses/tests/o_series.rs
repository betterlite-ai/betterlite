use betterlite_openai::reasoning_models::DropParams;
use serde_json::{Value, json};

use super::support;
use crate::responses::capabilities::{
    is_o_series_model, o_series_map_params, o_series_supported_params, supported_params,
};

fn mapped(params: &str, drop: DropParams) -> Value {
    let mapped = o_series_map_params("o3", &support::params(params), drop);
    serde_json::to_value(mapped).unwrap()
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

pub(super) fn o_series_params() {
    let base = supported_params();
    assert!(!base.contains("context_management"));
    assert!(base.contains("temperature"));
    let o_series = o_series_supported_params();
    assert!(!o_series.contains("temperature"));
    assert!(o_series.contains("top_p"));
    assert_eq!(o_series.len() + 1, base.len());
}

pub(super) fn o_series_mapping() {
    let params = r#"{"temperature":0.2,"top_p":0.9,"input":[{"role":"user","content":"x"}]}"#;
    let kept = mapped(params, DropParams::default());
    assert_eq!(kept["temperature"], json!(0.2));
    assert_eq!(kept["top_p"], json!(0.9));
    assert_eq!(kept["input"], json!([{"role": "user", "content": "x"}]));
    // a global or a request-level drop removes temperature and nothing else
    for drop in [
        DropParams {
            global: true,
            request: false,
        },
        DropParams {
            global: false,
            request: true,
        },
    ] {
        let dropped = mapped(params, drop);
        assert_eq!(dropped.get("temperature"), None);
        assert_eq!(dropped["top_p"], json!(0.9));
    }
    // a null temperature is present, so a drop removes it too
    let null_temperature = mapped(
        r#"{"temperature":null,"top_p":0.9}"#,
        DropParams {
            global: true,
            request: false,
        },
    );
    assert_eq!(null_temperature.get("temperature"), None);
    // without temperature the mapper copies every value
    let absent = mapped(
        r#"{"top_p":0.9}"#,
        DropParams {
            global: true,
            request: true,
        },
    );
    assert_eq!(absent, json!({"top_p": 0.9}));
}
