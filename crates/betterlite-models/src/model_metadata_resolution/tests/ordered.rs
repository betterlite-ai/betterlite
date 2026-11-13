use super::{Fields, Fixture, NeverDynamic, number_field, raw_field, resolve, string_field};

pub(super) async fn ordered_exact_lookup_case() {
    let fixture = ordered_fixture();
    candidate_order_case(&fixture).await;
    provider_match_case(&fixture).await;
    exact_before_generalization_case().await;
    providerless_entry_case().await;
    null_provider_entry_case().await;
    later_candidate_order_case().await;
}

async fn null_provider_entry_case() {
    let fixture =
        Fixture::raw(r#"{"vertex_ai/null-provider": {"litellm_provider": null, "mode": "chat"}}"#);
    let value = resolve(
        &fixture,
        "vertex_ai/null-provider",
        Some("vertex_ai"),
        &NeverDynamic,
    )
    .await
    .unwrap();
    assert_eq!(string_field(&value, "key"), "vertex_ai/null-provider");
    assert_eq!(raw_field(&value, "litellm_provider"), "null");
}

fn ordered_fixture() -> Fixture {
    Fixture::new(
        r#"{
            "groq/llama3-8b-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1},
            "groq/llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 2},
            "llama3-8b-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 3},
            "llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 4},
            "groq/groq/llama3-8b-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 5},
            "plain-model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 6},
            "split-only-model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 7},
            "groq/dated-only": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 8},
            "solo": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 9},
            "groq/groq/groq-target": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 10},
            "groq/mismatch-model": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 11},
            "mismatch-model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 12},
            "Groq/MiXeD-Case": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 14}
        }"#,
    )
}

async fn candidate_order_case(fixture: &Fixture) {
    let combined = lookup(fixture, "groq/llama3-8b-2024-01-01").await;
    assert_eq!(string_field(&combined, "key"), "groq/llama3-8b-2024-01-01");
    assert_eq!(number_field(&combined, "input_cost_per_token"), Some(1));

    let plain = lookup(fixture, "plain-model").await;
    assert_eq!(string_field(&plain, "key"), "plain-model");
    let split = lookup(fixture, "groq/split-only-model").await;
    assert_eq!(string_field(&split, "key"), "split-only-model");
    let stripped_combination = lookup(fixture, "groq/dated-only-2024-02-02").await;
    assert_eq!(
        string_field(&stripped_combination, "key"),
        "groq/dated-only"
    );
    let stripped = lookup(fixture, "solo-2024-04-04").await;
    assert_eq!(string_field(&stripped, "key"), "solo");
    let prefixed = lookup(fixture, "groq/groq-target").await;
    assert_eq!(string_field(&prefixed, "key"), "groq/groq/groq-target");
}

async fn provider_match_case(fixture: &Fixture) {
    let mismatch = lookup(fixture, "mismatch-model").await;
    assert_eq!(string_field(&mismatch, "key"), "mismatch-model");
    let casefolded = lookup(fixture, "groq/mixed-case").await;
    assert_eq!(string_field(&casefolded, "key"), "Groq/MiXeD-Case");
}

async fn exact_before_generalization_case() {
    let rules = r#"[{"pattern": "^groq/", "model_info": {"input_cost_per_token": 99}}]"#;
    let fixture = Fixture::with_rules(
        r#"{"exact-model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 13}}"#,
        "{}",
        rules,
    );
    let exact = lookup(&fixture, "exact-model").await;
    assert_eq!(number_field(&exact, "input_cost_per_token"), Some(13));
}

async fn providerless_entry_case() {
    let fixture = Fixture::new(r#"{"bare-entry": {"mode": "chat"}}"#);
    let bare = resolve(&fixture, "bare-entry", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&bare, "litellm_provider"), "null");
}

async fn later_candidate_order_case() {
    let fixture = Fixture::new(
        r#"{
            "groq/duo": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1},
            "duo": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 2},
            "target": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 3},
            "groq/groq/target-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 4},
            "split-pair-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 7},
            "groq/split-pair": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 8},
            "converse/x": {"litellm_provider": "bedrock", "mode": "chat", "input_cost_per_token": 5},
            "x": {"litellm_provider": "bedrock", "mode": "chat", "input_cost_per_token": 6}
        }"#,
    );
    let combined_stripped = lookup(&fixture, "duo-2024-01-01").await;
    assert_eq!(string_field(&combined_stripped, "key"), "groq/duo");
    let stripped = lookup(&fixture, "groq/target-2024-01-01").await;
    assert_eq!(string_field(&stripped, "key"), "target");
    let split = lookup(&fixture, "groq/split-pair-2024-01-01").await;
    assert_eq!(string_field(&split, "key"), "split-pair-2024-01-01");
    let model = resolve(&fixture, "converse/x", Some("bedrock"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&model, "key"), "converse/x");
}

async fn lookup(fixture: &Fixture, model: &str) -> Fields {
    resolve(fixture, model, Some("groq"), &NeverDynamic)
        .await
        .unwrap()
}
