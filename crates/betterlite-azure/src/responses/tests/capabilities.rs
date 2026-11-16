use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_params::error::ResponseParamsError;
use betterlite_openai::response_params::params::ResponsesOptions;
use betterlite_prompts::arguments::json::Json;

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

fn azure_options(model: &str, params: &str) -> ResponsesOptions {
    support::options(model, ProviderId::Azure, params)
}

fn mapped_json(options: &ResponsesOptions, drop: DropParams) -> Json {
    support::to_json(&support::mapped(options, drop).unwrap())
}

fn rejects_temperature(options: &ResponsesOptions) -> bool {
    matches!(
        support::mapped(options, NO_DROP),
        Err(ResponseParamsError::UnsupportedTemperature { .. })
    )
}

pub(super) fn azure_responses_capabilities() {
    let base = supported_params();
    let removed: Vec<&str> = support::supported().difference(&base).copied().collect();
    assert_eq!(removed, ["context_management"]);
    let o_series = o_series_supported_params();
    let narrowed: Vec<&str> = base.difference(&o_series).copied().collect();
    assert_eq!(narrowed, ["temperature"]);
    gated_sampling();
    passed_sampling();
    o_series_mapping();
}

fn gated_sampling() {
    // the Azure config skips the OpenAI-only reasoning gate, so a provider that would reject it maps
    let openai_provider = support::options(
        "gpt-5-unknown",
        ProviderId::Openai,
        r#"{"reasoning":{"effort":"high"}}"#,
    );
    assert_eq!(
        mapped_json(&openai_provider, NO_DROP),
        support::json(r#"{"reasoning":{"effort":"high"}}"#)
    );
    // an active effort rejects a non-default temperature
    let rejected = azure_options(
        "gpt-5",
        r#"{"temperature":0.2,"reasoning":{"effort":"high"},"top_p":0.9}"#,
    );
    assert!(rejects_temperature(&rejected));
    // dropping it removes temperature, and then top_p falls to the same check
    assert_eq!(
        mapped_json(&rejected, GLOBAL_DROP),
        support::json(r#"{"reasoning":{"effort":"high"}}"#)
    );
    // an entry without none support rejects the temperature of an active effort
    let plain = azure_options(
        "gpt-5-plain",
        r#"{"temperature":0.2,"reasoning":{"effort":"high"}}"#,
    );
    assert!(rejects_temperature(&plain));
    // a label-less entry falls back on the catalogue's declared default, so the sampling stays gated
    let unlabeled = azure_options("gpt-5-nolabel", r#"{"temperature":0.2}"#);
    assert!(rejects_temperature(&unlabeled));
    // and a catalogue with no declared default lets the none-support flag answer, keeping the sampling
    let mut no_default = support::fixture_with(support::NO_DEFAULT_MODEL_COST);
    let kept = support::mapped_with(&mut no_default, &unlabeled, NO_DROP).unwrap();
    assert_eq!(
        support::to_json(&kept),
        support::json(r#"{"temperature":0.2}"#)
    );
}

fn passed_sampling() {
    // an effort of none keeps the sampling parameters
    let none = azure_options(
        "gpt-5",
        r#"{"temperature":0.2,"reasoning":{"effort":"none"}}"#,
    );
    assert_eq!(
        mapped_json(&none, NO_DROP),
        support::json(r#"{"temperature":0.2,"reasoning":{"effort":"none"}}"#)
    );
    // the catalog's declared default effort answers when the request omits one
    let defaulted = azure_options("gpt-5", r#"{"temperature":1,"top_p":0.5}"#);
    assert_eq!(
        mapped_json(&defaulted, NO_DROP),
        support::json(r#"{"temperature":1,"top_p":0.5}"#)
    );
    // a prefixed model is not the bare gpt-5 family, so its sampling passes through
    let prefixed = azure_options("azure/gpt-5", r#"{"temperature":0.2,"top_p":0.9}"#);
    assert_eq!(
        mapped_json(&prefixed, NO_DROP),
        support::json(r#"{"temperature":0.2,"top_p":0.9}"#)
    );
    // the shared minimum output-token floor still applies
    let floor = azure_options("azure/gpt-5", r#"{"max_output_tokens":4}"#);
    assert_eq!(
        mapped_json(&floor, NO_DROP),
        support::json(r#"{"max_output_tokens":16}"#)
    );
}

fn o_series_mapping() {
    // the O-series mapper copies values and only drops temperature when allowed
    let params = support::params(r#"{"temperature":0.2,"top_p":0.9}"#);
    assert_eq!(
        support::to_json(&o_series_map_params("o3", &params, NO_DROP)),
        support::json(r#"{"temperature":0.2,"top_p":0.9}"#)
    );
    assert_eq!(
        support::to_json(&o_series_map_params("o3", &params, GLOBAL_DROP)),
        support::json(r#"{"top_p":0.9}"#)
    );
}
