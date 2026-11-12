#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

use super::Call;

const VARIANTS: [&str; 15] = [
    "gpt5_search_allowlist",
    "gpt5_chat_exclusion",
    "gpt6_family_selection",
    "dict_effort_normalization",
    "xhigh_opt_in",
    "low_minimal_opt_out",
    "catalog_before_default_key",
    "temperature_while_thinking",
    "sampling_with_none",
    "o1_developer_roles",
    "audio_token_parameter",
    "gpt5_regular_allowlist",
    "version_boundaries",
    "o_series_capability_fallback",
    "o_series_temperature_limit",
];

#[test]
fn helper_seeds_select_openai_reasoning_models() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_reasoning_models"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(
        !seeds.is_empty(),
        "the openai_reasoning_models seeds are missing"
    );
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        let fields = case.call.input.as_object().unwrap();
        let variant = fields.keys().next().unwrap();
        case.call.operation == "openai_reasoning_models"
            && fields.len() == 1
            && VARIANTS.contains(&variant.as_str())
            && serde_json::from_value::<Call>(case.call.input.clone()).is_ok_and(|call| {
                serde_json::to_value(call)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .is_some_and(|fields| fields.keys().next() == Some(variant))
            })
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
