#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use super::{
    CatalogGeneralizations, MAX_RULES_BYTES, ModelInfo, decode_model_generalization_rules,
};
use crate::error::ModelsError;

const INSTALL_RULES: &str = r#"[{"name":"base","pattern":"^zzz$","model_info":{"mode":"chat","max_tokens":1}},{"name":"base","pattern":"^yyy$","model_info":{"mode":"embedding","max_tokens":2}},{"name":"child","pattern":"^child","extends":"base","model_info":{"max_tokens":9}},{"name":"grand","pattern":"^grand","extends":"child","model_info":{"x":1}},{"pattern":"^orphan","extends":"missing","model_info":{"y":1}},{"pattern":"^numeric","extends":7,"model_info":{"z":1}},{"pattern":"^bad","extends":"base","model_info":"text"},"loose"]"#;
const CLASSIFY_RULES: &str = r#"[5,{"pattern":"^cap$","model_info":{}},{"pattern":"route","model_info":{"litellm_provider":"openai"}},{"pattern":"both","model_info":{"litellm_provider":"azure","mode":"chat"},"description":"ignored"},{"pattern":"^fill$","model_info":{"mode":"chat"},"fill_missing_for_providers":[]},{"pattern":"^FLAG$","model_info":{"mode":"x"},"fill_missing_for_providers":["openai","openai"]},{"pattern":"nonstr","model_info":{"litellm_provider":1,"mode":"chat"}}]"#;
const MALFORMED_RULES: &str = r#"[{"name":"no-pattern","model_info":{}},{"pattern":"p1","model_info":[]},{"model_info":{}},{"name":null,"pattern":"p2"},{"name":3,"pattern":5,"model_info":{}},{"pattern":"(","model_info":{},"fill_missing_for_providers":5},{"pattern":"it's(","model_info":{}},{"name":"fills","pattern":"f","model_info":{},"fill_missing_for_providers":"openai"},{"pattern":"f2","model_info":{},"fill_missing_for_providers":[1]},{"pattern":"prov","model_info":{"litellm_provider":null}},{"pattern":"ok","model_info":{"mode":"chat"}}]"#;
const ROUTING_RULES: &str = r#"[{"pattern":"^gpt-","model_info":{"litellm_provider":"openai"}},{"pattern":"gpt","model_info":{"litellm_provider":"later"}},{"pattern":"^blank$","model_info":{"litellm_provider":""}},{"pattern":"","model_info":{"litellm_provider":"anything"}}]"#;
const UNION_RULES: &str = r#"[{"pattern":"claude","model_info":{"mode":"chat","supports_vision":true,"max_tokens":100}},{"pattern":"claude-x","model_info":{"supports_vision":false,"max_tokens":null}},{"pattern":"legacy","model_info":{"litellm_provider":"bedrock","mode":"completion"}},{"pattern":"^empty$","model_info":{}}]"#;
const FILL_RULES: &str = r#"[{"pattern":"gemini","model_info":{"litellm_provider":"vertex_ai","supports_pdf":true},"fill_missing_for_providers":["vertex_ai"]},{"pattern":"gemini-3","model_info":{"supports_pdf":false,"max_tokens":null},"fill_missing_for_providers":["vertex_ai","gemini"]},{"pattern":"^bare$","model_info":{},"fill_missing_for_providers":["openai"]},{"pattern":"^route$","model_info":{"litellm_provider":"openai"},"fill_missing_for_providers":["openai"]},{"pattern":"^plain$","model_info":{"mode":"chat"}}]"#;
const MALFORMED: &str = "(needs string 'pattern' and dict 'model_info').";
const BAD_FILL: &str = "('fill_missing_for_providers' must be a list of provider strings).";

enum Probe {
    Initial,
    NonList(&'static str),
    Rules,
    Compiled,
    Warnings,
    Routing(&'static str),
    Capabilities(&'static str),
    FillMissing(&'static str, &'static str),
}

fn info_text(info: Option<ModelInfo>) -> String {
    info.map_or_else(
        || "None".to_owned(),
        |fields| serde_json::to_string(&fields).unwrap(),
    )
}

#[rstest]
#[case::install_and_legacy_extends(INSTALL_RULES, &[Probe::Initial, Probe::NonList(r#"{"rules":[]}"#), Probe::NonList("null"), Probe::Rules, Probe::Warnings, Probe::Capabilities("child-1"), Probe::Capabilities("grand-1"), Probe::Capabilities("orphan"), Probe::Capabilities("numeric"), Probe::Capabilities("yyy")], &[
    "initial 0 0 0",
    "non-list 0",
    "non-list 0",
    INSTALL_RULES,
    &format!("LiteLLM: skipping malformed fallback generalization rule ^bad {MALFORMED}"),
    r#"child-1 -> {"max_tokens":9,"mode":"embedding"}"#,
    r#"grand-1 -> {"max_tokens":9,"x":1}"#,
    r#"orphan -> {"y":1}"#,
    r#"numeric -> {"z":1}"#,
    r#"yyy -> {"max_tokens":2,"mode":"embedding"}"#,
])]
#[case::compile_classification(CLASSIFY_RULES, &[Probe::Compiled, Probe::Warnings, Probe::Routing("Route-me"), Probe::Routing("xBOTHx"), Probe::Routing("cap"), Probe::Capabilities("CAP"), Probe::Capabilities("both"), Probe::Capabilities("route"), Probe::Capabilities("nonstr"), Probe::FillMissing("flag", "openai"), Probe::FillMissing("fill", "openai")], &[
    "routing 2 capabilities 4 flagged 1",
    "LiteLLM: skipping invalid fallback generalization rule nonstr: 'litellm_provider' in 'model_info' must be a string.",
    r#"Route-me -> Some("openai")"#,
    r#"xBOTHx -> Some("azure")"#,
    "cap -> None",
    "CAP -> {}",
    r#"both -> {"litellm_provider":"azure","mode":"chat"}"#,
    "route -> None",
    "nonstr -> None",
    r#"flag/openai -> {"mode":"x"}"#,
    "fill/openai -> None",
])]
#[case::malformed_rule_warnings(MALFORMED_RULES, &[Probe::Warnings, Probe::Compiled], &[
    &format!("LiteLLM: skipping malformed fallback generalization rule no-pattern {MALFORMED}"),
    &format!("LiteLLM: skipping malformed fallback generalization rule p1 {MALFORMED}"),
    &format!("LiteLLM: skipping malformed fallback generalization rule None {MALFORMED}"),
    &format!("LiteLLM: skipping malformed fallback generalization rule None {MALFORMED}"),
    &format!("LiteLLM: skipping malformed fallback generalization rule 3 {MALFORMED}"),
    "LiteLLM: skipping fallback generalization rule with invalid regex '(': Parsing error at position 1: Opening parenthesis without closing parenthesis",
    "LiteLLM: skipping fallback generalization rule with invalid regex \"it's(\": Parsing error at position 5: Opening parenthesis without closing parenthesis",
    &format!("LiteLLM: skipping malformed fallback generalization rule fills {BAD_FILL}"),
    &format!("LiteLLM: skipping malformed fallback generalization rule f2 {BAD_FILL}"),
    "LiteLLM: skipping invalid fallback generalization rule prov: 'litellm_provider' in 'model_info' must be a string.",
    "routing 0 capabilities 1 flagged 0",
])]
#[case::first_routing_match(ROUTING_RULES, &[Probe::Routing("GPT-4o"), Probe::Routing("my-gpt-4"), Probe::Routing("BLANK"), Probe::Routing("other"), Probe::Routing("")], &[
    r#"GPT-4o -> Some("openai")"#,
    r#"my-gpt-4 -> Some("later")"#,
    r#"BLANK -> Some("")"#,
    r#"other -> Some("anything")"#,
    " -> None",
])]
#[case::capability_union(UNION_RULES, &[Probe::Capabilities("claude-x-1"), Probe::Capabilities("legacy-claude"), Probe::Capabilities("claude"), Probe::Capabilities("EMPTY"), Probe::Capabilities("gpt"), Probe::Capabilities("")], &[
    r#"claude-x-1 -> {"max_tokens":null,"mode":"chat","supports_vision":false}"#,
    r#"legacy-claude -> {"litellm_provider":"bedrock","max_tokens":100,"mode":"completion","supports_vision":true}"#,
    r#"claude -> {"max_tokens":100,"mode":"chat","supports_vision":true}"#,
    "EMPTY -> {}",
    "gpt -> None",
    " -> None",
])]
#[case::fill_missing_projection(FILL_RULES, &[Probe::FillMissing("gemini-3-pro", "vertex_ai"), Probe::FillMissing("Gemini-2", "vertex_ai"), Probe::FillMissing("gemini-3", "gemini"), Probe::FillMissing("gemini-2", "Vertex_AI"), Probe::FillMissing("gemini-2", ""), Probe::FillMissing("", "vertex_ai"), Probe::FillMissing("bare", "openai"), Probe::FillMissing("route", "openai"), Probe::FillMissing("plain", "openai")], &[
    r#"gemini-3-pro/vertex_ai -> {"max_tokens":null,"supports_pdf":false}"#,
    r#"Gemini-2/vertex_ai -> {"supports_pdf":true}"#,
    r#"gemini-3/gemini -> {"max_tokens":null,"supports_pdf":false}"#,
    "gemini-2/Vertex_AI -> None",
    "gemini-2/ -> None",
    "/vertex_ai -> None",
    "bare/openai -> None",
    "route/openai -> None",
    "plain/openai -> None",
])]
fn catalog_generalizations_cases(
    #[case] rules: &str,
    #[case] probes: &[Probe],
    #[case] expected: &[&str],
) {
    let (generalizations, warnings) = CatalogGeneralizations::install(
        decode_model_generalization_rules(rules.as_bytes()).unwrap(),
    );
    let mut lines = Vec::new();
    for probe in probes {
        match *probe {
            Probe::Initial => {
                let initial = CatalogGeneralizations::default();
                lines.push(format!(
                    "initial {} {} {}",
                    initial.rules.len(),
                    initial.routing.len(),
                    initial.capabilities.len()
                ));
            }
            Probe::NonList(text) => {
                lines.push(format!(
                    "non-list {}",
                    decode_model_generalization_rules(text.as_bytes())
                        .unwrap()
                        .len()
                ));
            }
            Probe::Rules => lines.push(serde_json::to_string(&generalizations.rules).unwrap()),
            Probe::Compiled => {
                let capabilities = &generalizations.capabilities;
                let flagged = capabilities
                    .iter()
                    .filter(|rule| !rule.fill_missing_for_providers.is_empty())
                    .count();
                lines.push(format!(
                    "routing {} capabilities {} flagged {flagged}",
                    generalizations.routing.len(),
                    capabilities.len()
                ));
            }
            Probe::Warnings => lines.extend(warnings.iter().map(ToString::to_string)),
            Probe::Routing(model) => lines.push(format!(
                "{model} -> {:?}",
                generalizations.match_routing(model)
            )),
            Probe::Capabilities(model) => lines.push(format!(
                "{model} -> {}",
                info_text(generalizations.match_capabilities(model))
            )),
            Probe::FillMissing(model, provider) => {
                lines.push(format!(
                    "{model}/{provider} -> {}",
                    info_text(generalizations.match_fill_missing(model, provider))
                ));
            }
        }
    }
    assert_eq!(lines, expected);
}

#[rstest]
#[case::at_limit(MAX_RULES_BYTES, "rules 0")]
#[case::over_limit(MAX_RULES_BYTES + 1, "the generalization rule list is 1048577 bytes, over the 1048576 byte limit")]
fn rule_lists_are_bounded(#[case] size: usize, #[case] expected: &str) {
    let mut bytes = vec![b' '; size];
    bytes.splice(..2, *b"[]");
    let outcome = decode_model_generalization_rules(&bytes);
    let described = outcome.map_or_else(
        |error| error.to_string(),
        |rules| format!("rules {}", rules.len()),
    );
    assert_eq!(described, expected);
}

#[test]
fn replays_the_model_generalization_rules_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/model_generalization_rules");
    let names: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let category = match decode_model_generalization_rules(&fs::read(&path).unwrap()) {
                Ok(rules) if rules.is_empty() => "empty",
                Ok(rules) => {
                    let (generalizations, _warnings) = CatalogGeneralizations::install(rules);
                    assert_eq!(
                        generalizations.match_routing("claude-x-1"),
                        generalizations.match_routing("CLAUDE-x-1")
                    );
                    drop(generalizations.match_capabilities(&name));
                    drop(generalizations.match_fill_missing(&name, "anthropic"));
                    "rules"
                }
                Err(ModelsError::RulesJson { .. }) => "json",
                Err(error) => panic!("{name} failed outside the decoder categories: {error}"),
            };
            assert!(name.starts_with(category), "{name} decoded as {category}");
            name
        })
        .collect();
    assert!(!names.is_empty());
}
