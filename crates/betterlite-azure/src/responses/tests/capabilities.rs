use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_params::error::ResponseParamsError;
use serde_json::{Value, json};

use super::support;
use crate::responses::capabilities::{
    o_series_map_params, o_series_supported_params, supported_params,
};

const NO_DROP: DropParams = DropParams {
    global: false,
    request: false,
};
const GLOBAL_DROP: DropParams = DropParams {
    global: true,
    request: false,
};

fn azure_options(
    model: &str,
    params: &str,
) -> betterlite_openai::response_params::params::ResponsesOptions {
    support::options(model, ProviderId::Azure, params)
}

pub(super) fn azure_responses_capabilities() {
    let base = supported_params();
    assert!(!base.contains("context_management"));
    assert_eq!(base.len() + 1, support::supported().len());
    let o_series = o_series_supported_params();
    assert!(!o_series.contains("temperature"));
    assert_eq!(o_series.len() + 1, base.len());
    mapping();
}

fn mapping() {
    // the Azure config skips the OpenAI-only reasoning gate, so a provider that would reject it maps
    let openai_provider = support::options(
        "gpt-5-unknown",
        ProviderId::Openai,
        r#"{"reasoning":{"effort":"high"}}"#,
    );
    assert_eq!(
        support::mapped(&openai_provider, NO_DROP).unwrap(),
        json!({"reasoning": {"effort": "high"}})
    );
    // an active effort rejects a non-default temperature
    let rejected = azure_options(
        "gpt-5",
        r#"{"temperature":0.2,"reasoning":{"effort":"high"},"top_p":0.9}"#,
    );
    let error = support::mapped(&rejected, NO_DROP).unwrap_err();
    assert!(matches!(
        error,
        ResponseParamsError::UnsupportedTemperature { .. }
    ));
    // dropping it removes temperature, and then top_p falls to the same check
    assert_eq!(
        support::mapped(&rejected, GLOBAL_DROP).unwrap(),
        json!({"reasoning": {"effort": "high"}})
    );
    // an effort of none keeps the sampling parameters
    let none = azure_options(
        "gpt-5",
        r#"{"temperature":0.2,"reasoning":{"effort":"none"}}"#,
    );
    assert_eq!(
        support::mapped(&none, NO_DROP).unwrap(),
        json!({"temperature": 0.2, "reasoning": {"effort": "none"}})
    );
    // the catalog's declared default effort answers when the request omits one
    let defaulted = azure_options("gpt-5", r#"{"temperature":1,"top_p":0.5}"#);
    assert_eq!(
        support::mapped(&defaulted, NO_DROP).unwrap(),
        json!({"temperature": 1, "top_p": 0.5})
    );
    // an entry without none support rejects the temperature of an active effort
    let plain = azure_options(
        "gpt-5-plain",
        r#"{"temperature":0.2,"reasoning":{"effort":"high"}}"#,
    );
    assert!(matches!(
        support::mapped(&plain, NO_DROP).unwrap_err(),
        ResponseParamsError::UnsupportedTemperature { .. }
    ));
    // a prefixed model is not the bare gpt-5 family, so its sampling passes through
    let prefixed = azure_options("azure/gpt-5", r#"{"temperature":0.2,"top_p":0.9}"#);
    assert_eq!(
        support::mapped(&prefixed, NO_DROP).unwrap(),
        json!({"temperature": 0.2, "top_p": 0.9})
    );
    // the shared minimum output-token floor still applies
    let floor = azure_options("azure/gpt-5", r#"{"max_output_tokens":4}"#);
    assert_eq!(
        support::mapped(&floor, NO_DROP).unwrap(),
        json!({"max_output_tokens": 16})
    );
    // the O-series mapper copies values and only drops temperature when allowed
    let params = support::params(r#"{"temperature":0.2,"top_p":0.9}"#);
    let kept: Value = serde_json::to_value(o_series_map_params("o3", &params, NO_DROP)).unwrap();
    assert_eq!(kept, json!({"temperature": 0.2, "top_p": 0.9}));
    let dropped: Value =
        serde_json::to_value(o_series_map_params("o3", &params, GLOBAL_DROP)).unwrap();
    assert_eq!(dropped, json!({"top_p": 0.9}));
}
