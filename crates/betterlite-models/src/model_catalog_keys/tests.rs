#![cfg(test)]

use std::collections::HashMap;

use indexmap::IndexMap;
use rstest::rstest;

use super::{
    EntryProvider, ModelCatalogKeysError, ModelCostKeys, PotentialModelNames, check_provider_match,
    get_model_info_from_model_cost, strip_dated_snapshot_suffix, strip_finetune_model_name,
    strip_model_name, strip_stable_vertex_version,
};

fn bedrock_marker(model: &str) -> String {
    format!("bedrock:{model}")
}

fn catalog(keys: &[&str]) -> IndexMap<String, u32> {
    keys.iter()
        .zip(1_u32..)
        .map(|(key, value)| ((*key).to_owned(), value))
        .collect()
}

fn names(split: &str, stripped: &str, provider: Option<&str>) -> PotentialModelNames {
    PotentialModelNames {
        split_model: split.to_owned(),
        combined_model_name: "combined".to_owned(),
        stripped_model_name: stripped.to_owned(),
        combined_stripped_model_name: "combined_stripped".to_owned(),
        provider_prefixed_model_name: "prefixed".to_owned(),
        custom_llm_provider: provider.map(str::to_owned),
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    ModelNormalization,
    ExactAndCasefoldKeys,
    CatalogInvalidation,
    ProviderMatching,
    DirectAndPotentialLookup,
}

fn model_normalization() {
    let bedrock = |model, provider| strip_model_name(model, provider, bedrock_marker);
    assert_eq!(bedrock("us.llama", Some("bedrock")), "bedrock:us.llama");
    assert_eq!(
        bedrock("m-2024-01-02", Some("bedrock_converse")),
        "bedrock:m-2024-01-02"
    );
    assert_eq!(bedrock("gemini-1-002", Some("vertex_ai")), "gemini-1");
    assert_eq!(bedrock("gemini-1-002", Some("gemini")), "gemini-1");
    assert_eq!(bedrock("llama-3", Some("databricks")), "llama");
    assert_eq!(bedrock("ft:gpt-4o:org:suffix:id", None), "ft:gpt-4o");
    assert_eq!(bedrock("gpt-4o-2024-08-06", Some("openai")), "gpt-4o");
    assert_eq!(bedrock("gpt-4o-2024-08-06", None), "gpt-4o");
    assert_eq!(
        bedrock("ft:gpt-4o-2024-08-06", Some("vertex_ai")),
        "ft:gpt-4o-2024-08"
    );
}

fn exact_and_casefold_keys() {
    let mut model_cost = catalog(&["Foo", "foo", "Bar", "BAR", "baz"]);
    let mut keys = ModelCostKeys::default();
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "Foo").as_deref(),
        Some("Foo")
    );
    assert_eq!(keys.lowercase_index, None);
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "fOO").as_deref(),
        Some("foo")
    );
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "bar").as_deref(),
        Some("BAR")
    );
    model_cost.shift_remove("BAR");
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "bar").as_deref(),
        Some("Bar")
    );
    model_cost.shift_remove("Bar");
    assert_eq!(keys.get_model_cost_key(&model_cost, "BAR"), None);
    assert_eq!(keys.get_model_cost_key(&model_cost, "bar"), None);
    model_cost.insert("QUX".to_owned(), 9);
    assert_eq!(keys.get_model_cost_key(&model_cost, "qux"), None);
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "QUX").as_deref(),
        Some("QUX")
    );
    let index = keys.lowercase_index.as_ref().unwrap();
    assert!(!index.contains_key("qux"));
    assert_eq!(index.get("baz").map(String::as_str), Some("baz"));
}

fn catalog_invalidation() {
    let mut model_cost = catalog(&["Alpha"]);
    let mut keys = ModelCostKeys::default();
    let mut full = HashMap::from([("a", 1)]);
    let mut helper = HashMap::from([("b", 2)]);
    assert_eq!(keys.generation, 0);
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "alpha").as_deref(),
        Some("Alpha")
    );
    assert!(keys.lowercase_index.is_some());
    model_cost.insert("Beta".to_owned(), 2);
    assert_eq!(keys.get_model_cost_key(&model_cost, "beta"), None);
    keys.invalidate(&mut full, &mut helper);
    assert_eq!(keys.generation, 1);
    assert_eq!(keys.lowercase_index, None);
    assert!(full.is_empty() && helper.is_empty());
    assert_eq!(
        keys.get_model_cost_key(&model_cost, "beta").as_deref(),
        Some("Beta")
    );
    keys.invalidate(&mut full, &mut helper);
    assert_eq!(keys.generation, 2);
}

fn provider_matching() {
    let matches = |entry, provider| check_provider_match(entry, provider).unwrap();
    assert!(matches(EntryProvider::Text("openai"), None));
    assert!(matches(EntryProvider::Unconstrained, Some("openai")));
    assert!(matches(EntryProvider::Text("openai"), Some("openai")));
    assert!(!matches(EntryProvider::Text("openai"), Some("azure")));
    assert!(!matches(EntryProvider::Text("openai"), Some("")));
    assert!(matches(EntryProvider::Text("openai"), Some("azure_ai")));
    assert!(matches(EntryProvider::Text("azure"), Some("azure_ai")));
    assert!(!matches(EntryProvider::Text("cohere"), Some("azure_ai")));
    assert!(!matches(EntryProvider::NotText, Some("azure_ai")));
    assert!(matches(EntryProvider::NotText, Some("github")));
    assert!(matches!(
        check_provider_match(EntryProvider::NotText, Some("bedrock_converse")),
        Err(ModelCatalogKeysError::ProviderNotText)
    ));
    assert!(check_provider_match(EntryProvider::NotText, Some("vertex_ai")).is_err());
    assert!(check_provider_match(EntryProvider::NotText, Some("fireworks_ai")).is_err());
}

fn direct_and_potential_lookup() {
    let model_cost = catalog(&["Known", "bedrock", "None"]);
    let mut keys = ModelCostKeys::default();
    assert_eq!(
        get_model_info_from_model_cost(&model_cost, "Known").unwrap(),
        &1
    );
    assert!(matches!(
        get_model_info_from_model_cost(&model_cost, "known"),
        Err(ModelCatalogKeysError::MissingModel { key }) if key == "known"
    ));
    let present =
        |keys: &mut ModelCostKeys, names| keys.has_potential_model_name(&model_cost, &names);
    assert!(!present(&mut keys, names("a", "c", Some("d"))));
    assert!(present(&mut keys, names("a", "c", Some("BEDROCK"))));
    assert!(present(&mut keys, names("KNOWN", "c", Some("d"))));
    assert!(present(&mut keys, names("a", "known", Some("d"))));
    assert!(present(&mut keys, names("a", "c", None)));
    let mut prefixed = names("a", "c", Some("d"));
    prefixed.provider_prefixed_model_name = "known".to_owned();
    assert!(present(&mut keys, prefixed));
    let mut combined = names("a", "c", Some("d"));
    combined.combined_model_name = "known".to_owned();
    assert!(present(&mut keys, combined));
    let mut combined_stripped = names("a", "c", Some("d"));
    combined_stripped.combined_stripped_model_name = "known".to_owned();
    assert!(present(&mut keys, combined_stripped));
}

#[rstest]
#[case::model_normalization(Scenario::ModelNormalization)]
#[case::exact_and_casefold_keys(Scenario::ExactAndCasefoldKeys)]
#[case::catalog_invalidation(Scenario::CatalogInvalidation)]
#[case::provider_matching(Scenario::ProviderMatching)]
#[case::direct_and_potential_lookup(Scenario::DirectAndPotentialLookup)]
fn model_catalog_keys_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ModelNormalization => model_normalization(),
        Scenario::ExactAndCasefoldKeys => exact_and_casefold_keys(),
        Scenario::CatalogInvalidation => catalog_invalidation(),
        Scenario::ProviderMatching => provider_matching(),
        Scenario::DirectAndPotentialLookup => direct_and_potential_lookup(),
    }
}

#[rstest]
#[case::trailing_digits("a-1", "a")]
#[case::unicode_digits("a-١٢", "a")]
#[case::no_hyphen("a1", "a1")]
#[case::hyphen_only("a-", "a-")]
#[case::inner_digits("a-1-b", "a-1-b")]
#[case::trailing_newline("a-1\n", "a\n")]
#[case::empty("", "")]
fn vertex_version_strip(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(strip_stable_vertex_version(model), expected);
}

#[rstest]
#[case::dated("gpt-4-2024-08-06", "gpt-4")]
#[case::trailing_newline("gpt-4-2024-08-06\n", "gpt-4\n")]
#[case::bare_date("-2024-08-06", "")]
#[case::short_year("gpt-24-08-06", "gpt-24-08-06")]
#[case::suffix_text("gpt-2024-08-06-x", "gpt-2024-08-06-x")]
#[case::too_short("2024-08-06", "2024-08-06")]
#[case::wrong_dash("gpt_2024-08-06", "gpt_2024-08-06")]
#[case::letters("gpt-abcd-08-06", "gpt-abcd-08-06")]
#[case::multibyte("é-2024-08-06", "é")]
fn dated_snapshot_strip(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(strip_dated_snapshot_suffix(model), expected);
}

#[rstest]
#[case::full("ft:gpt-3.5-turbo:my-org:custom_suffix:id", "ft:gpt-3.5-turbo")]
#[case::empty_suffix("ft:gpt-4o-2024-08-06:my-org::id", "ft:gpt-4o-2024-08-06")]
#[case::two_colons("ft:a:b", "ft:a:b")]
#[case::extra_colons("ft:a:b:c:d:e", "ft:a:b")]
#[case::trailing_newline("ft:a:b:c:d\n", "ft:a")]
fn finetune_strip(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(strip_finetune_model_name(model), expected);
}

#[rstest]
#[case::finetune_substring("x-ft:y:a:b:c", Some("openai"), "x-ft:y")]
#[case::bedrock_wins_over_finetune("ft:y", Some("bedrock"), "bedrock:ft:y")]
#[case::vertex_wins_over_finetune("ft:y-1", Some("vertex_ai"), "ft:y")]
#[case::empty_provider_is_dated("m-2024-01-02", Some(""), "m")]
fn strip_model_name_branches(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(strip_model_name(model, provider, bedrock_marker), expected);
}

#[rstest]
#[case::vertex_family("vertex_ai", "vertex_ai_beta", true)]
#[case::vertex_other("vertex_ai", "gemini", false)]
#[case::fireworks_family("fireworks_ai", "fireworks_ai-embedding-models", true)]
#[case::fireworks_other("fireworks_ai", "openai", false)]
#[case::bedrock_family("bedrock", "bedrock_converse", true)]
#[case::bedrock_converse_family("bedrock_converse", "bedrock", true)]
#[case::bedrock_other("bedrock", "openai", false)]
#[case::proxy_accepts_all("litellm_proxy", "anthropic", true)]
#[case::nadir_accepts_all("nadir", "anthropic", true)]
#[case::github_accepts_all("github", "anthropic", true)]
#[case::azure_ai_accepts_openai("azure_ai", "openai", true)]
#[case::azure_ai_rejects_other("azure_ai", "anthropic", false)]
#[case::prefix_is_not_mutual("vertex_ai_beta", "vertex_ai", false)]
fn provider_mismatch_rules(#[case] requested: &str, #[case] entry: &str, #[case] expected: bool) {
    assert_eq!(
        check_provider_match(EntryProvider::Text(entry), Some(requested)).unwrap(),
        expected
    );
}
