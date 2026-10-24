#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use super::{CatalogGeneralizations, MAX_RULES_BYTES, decode_model_generalization_rules};
use crate::error::ModelsError;

const INSTALL_RULES: &str = r#"[{"name":"base","pattern":"^zzz$","model_info":{"mode":"chat","max_tokens":1}},{"name":"base","pattern":"^yyy$","model_info":{"mode":"embedding","max_tokens":2}},{"name":"child","pattern":"^child","extends":"base","model_info":{"max_tokens":9}},{"name":"grand","pattern":"^grand","extends":"child","model_info":{"x":1}},{"pattern":"^orphan","extends":"missing","model_info":{"y":1}},{"pattern":"^numeric","extends":7,"model_info":{"z":1}},{"pattern":"^bad","extends":"base","model_info":"text"},"loose"]"#;
const CLASSIFY_RULES: &str = r#"[5,{"pattern":"^cap$","model_info":{}},{"pattern":"route","model_info":{"litellm_provider":"openai"}},{"pattern":"both","model_info":{"litellm_provider":"azure","mode":"chat"},"description":"ignored"},{"pattern":"^fill$","model_info":{"mode":"chat"},"fill_missing_for_providers":[]},{"pattern":"^FLAG$","model_info":{"mode":"x"},"fill_missing_for_providers":["openai","openai"]},{"pattern":"nonstr","model_info":{"litellm_provider":1,"mode":"chat"}}]"#;
const MALFORMED_RULES: &str = r#"[{"name":"no-pattern","model_info":{}},{"pattern":"p1","model_info":[]},{"model_info":{}},{"name":null,"pattern":"p2"},{"name":3,"pattern":5,"model_info":{}},{"pattern":"(","model_info":{},"fill_missing_for_providers":5},{"pattern":"it's(","model_info":{}},{"name":"fills","pattern":"f","model_info":{},"fill_missing_for_providers":"openai"},{"pattern":"f2","model_info":{},"fill_missing_for_providers":[1]},{"pattern":"prov","model_info":{"litellm_provider":null}},{"pattern":"ok","model_info":{"mode":"chat"}}]"#;
const ROUTING_RULES: &str = r#"[{"pattern":"^gpt-","model_info":{"litellm_provider":"openai"}},{"pattern":"gpt","model_info":{"litellm_provider":"later"}},{"pattern":"^blank$","model_info":{"litellm_provider":""}},{"pattern":"","model_info":{"litellm_provider":"anything"}}]"#;
const UNION_RULES: &str = r#"[{"pattern":"claude","model_info":{"mode":"chat","supports_vision":true,"max_tokens":100}},{"pattern":"claude-x","model_info":{"supports_vision":false,"max_tokens":null}},{"pattern":"legacy","model_info":{"litellm_provider":"bedrock","mode":"completion"}},{"pattern":"^empty$","model_info":{}}]"#;
const FILL_RULES: &str = r#"[{"pattern":"gemini","model_info":{"litellm_provider":"vertex_ai","supports_pdf":true},"fill_missing_for_providers":["vertex_ai"]},{"pattern":"gemini-3","model_info":{"supports_pdf":false,"max_tokens":null},"fill_missing_for_providers":["vertex_ai","gemini"]},{"pattern":"^bare$","model_info":{},"fill_missing_for_providers":["openai"]},{"pattern":"^route$","model_info":{"litellm_provider":"openai"},"fill_missing_for_providers":["openai"]},{"pattern":"^plain$","model_info":{"mode":"chat"}}]"#;
const SKIP_MALFORMED: &str = "LiteLLM: skipping malformed fallback generalization rule";
const NEEDS: &str = "(needs string 'pattern' and dict 'model_info').";
const BAD_FILL: &str = "('fill_missing_for_providers' must be a list of provider strings).";
const BAD_PROVIDER: &str = "'litellm_provider' in 'model_info' must be a string.";
const BAD_REGEX: &str = "LiteLLM: skipping fallback generalization rule with invalid regex";
const UNCLOSED: &str =
    "Parsing error at position 1: Opening parenthesis without closing parenthesis";

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

#[rstest]
#[case::install_and_legacy_extends(INSTALL_RULES, &[Probe::Initial, Probe::NonList(r#"{"rules":[]}"#), Probe::NonList("null"), Probe::Rules, Probe::Warnings, Probe::Capabilities("child-1"), Probe::Capabilities("grand-1"), Probe::Capabilities("orphan"), Probe::Capabilities("numeric"), Probe::Capabilities("yyy")], &[
    "CatalogGeneralizations { rules: [], routing: [], capabilities: [] }", "[]", "[]", INSTALL_RULES, &format!("{SKIP_MALFORMED} ^bad {NEEDS}"),
    r#"child-1 {"max_tokens":9,"mode":"embedding"}"#, r#"grand-1 {"max_tokens":9,"x":1}"#, r#"orphan {"y":1}"#, r#"numeric {"z":1}"#, r#"yyy {"max_tokens":2,"mode":"embedding"}"#,
])]
#[case::compile_classification(CLASSIFY_RULES, &[Probe::Compiled, Probe::Warnings, Probe::Routing("Route-me"), Probe::Routing("xBOTHx"), Probe::Routing("cap"), Probe::Capabilities("CAP"), Probe::Capabilities("both"), Probe::Capabilities("route"), Probe::Capabilities("nonstr"), Probe::FillMissing("flag", "openai"), Probe::FillMissing("fill", "openai")], &[
    "routing 2 capabilities 4 flagged 1", &format!("LiteLLM: skipping invalid fallback generalization rule nonstr: {BAD_PROVIDER}"),
    r#"Route-me "openai""#, r#"xBOTHx "azure""#, "cap null", "CAP {}", r#"both {"litellm_provider":"azure","mode":"chat"}"#, "route null", "nonstr null", r#"flag/openai {"mode":"x"}"#, "fill/openai null",
])]
#[case::malformed_rule_warnings(MALFORMED_RULES, &[Probe::Warnings, Probe::Compiled], &[
    &format!("{SKIP_MALFORMED} no-pattern {NEEDS}"), &format!("{SKIP_MALFORMED} p1 {NEEDS}"), &format!("{SKIP_MALFORMED} None {NEEDS}"), &format!("{SKIP_MALFORMED} None {NEEDS}"), &format!("{SKIP_MALFORMED} 3 {NEEDS}"),
    &format!("{BAD_REGEX} '(': {UNCLOSED}"), &format!("{BAD_REGEX} \"it's(\": {}", UNCLOSED.replace('1', "5")), &format!("{SKIP_MALFORMED} fills {BAD_FILL}"), &format!("{SKIP_MALFORMED} f2 {BAD_FILL}"),
    &format!("LiteLLM: skipping invalid fallback generalization rule prov: {BAD_PROVIDER}"), "routing 0 capabilities 1 flagged 0",
])]
#[case::first_routing_match(ROUTING_RULES, &[Probe::Routing("GPT-4o"), Probe::Routing("my-gpt-4"), Probe::Routing("BLANK"), Probe::Routing("other"), Probe::Routing("")], &[
    r#"GPT-4o "openai""#, r#"my-gpt-4 "later""#, r#"BLANK """#, r#"other "anything""#, " null",
])]
#[case::capability_union(UNION_RULES, &[Probe::Capabilities("claude-x-1"), Probe::Capabilities("legacy-claude"), Probe::Capabilities("claude"), Probe::Capabilities("EMPTY"), Probe::Capabilities("gpt"), Probe::Capabilities("")], &[
    r#"claude-x-1 {"max_tokens":null,"mode":"chat","supports_vision":false}"#, r#"legacy-claude {"litellm_provider":"bedrock","max_tokens":100,"mode":"completion","supports_vision":true}"#,
    r#"claude {"max_tokens":100,"mode":"chat","supports_vision":true}"#, "EMPTY {}", "gpt null", " null",
])]
#[case::fill_missing_projection(FILL_RULES, &[Probe::FillMissing("gemini-3-pro", "vertex_ai"), Probe::FillMissing("Gemini-2", "vertex_ai"), Probe::FillMissing("gemini-3", "gemini"), Probe::FillMissing("gemini-2", "Vertex_AI"), Probe::FillMissing("gemini-2", ""), Probe::FillMissing("", "vertex_ai"), Probe::FillMissing("bare", "openai"), Probe::FillMissing("route", "openai"), Probe::FillMissing("plain", "openai")], &[
    r#"gemini-3-pro/vertex_ai {"max_tokens":null,"supports_pdf":false}"#, r#"Gemini-2/vertex_ai {"supports_pdf":true}"#, r#"gemini-3/gemini {"max_tokens":null,"supports_pdf":false}"#,
    "gemini-2/Vertex_AI null", "gemini-2/ null", "/vertex_ai null", "bare/openai null", "route/openai null", "plain/openai null",
])]
fn catalog_generalizations_cases(
    #[case] rules: &str,
    #[case] probes: &[Probe],
    #[case] expected: &[&str],
) {
    let decoded = decode_model_generalization_rules(rules.as_bytes()).unwrap();
    let (installed, warnings) = CatalogGeneralizations::install(decoded);
    let mut lines = Vec::new();
    for probe in probes {
        let json = match *probe {
            Probe::Initial => format!("{:?}", CatalogGeneralizations::default()),
            Probe::NonList(text) => format!(
                "{:?}",
                decode_model_generalization_rules(text.as_bytes()).unwrap()
            ),
            Probe::Rules => serde_json::to_string(&installed.rules).unwrap(),
            Probe::Compiled => {
                let flagged = installed
                    .capabilities
                    .iter()
                    .filter(|rule| !rule.fill_missing_for_providers.is_empty());
                format!(
                    "routing {} capabilities {} flagged {}",
                    installed.routing.len(),
                    installed.capabilities.len(),
                    flagged.count()
                )
            }
            Probe::Warnings => warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
            Probe::Routing(model) => format!(
                "{model} {}",
                serde_json::to_string(&installed.match_routing(model)).unwrap()
            ),
            Probe::Capabilities(model) => format!(
                "{model} {}",
                serde_json::to_string(&installed.match_capabilities(model)).unwrap()
            ),
            Probe::FillMissing(model, provider) => {
                let fill = installed.match_fill_missing(model, provider);
                format!(
                    "{model}/{provider} {}",
                    serde_json::to_string(&fill).unwrap()
                )
            }
        };
        lines.extend(json.lines().map(str::to_owned));
    }
    assert_eq!(lines, expected);
}

#[rstest]
#[case::at_limit(MAX_RULES_BYTES, "rules 0")]
#[case::over_limit(MAX_RULES_BYTES + 1, "the generalization rule list is 1048577 bytes, over the 1048576 byte limit")]
fn rule_lists_are_bounded(#[case] size: usize, #[case] expected: &str) {
    let mut bytes = vec![b' '; size];
    bytes.splice(..2, *b"[]");
    let described = match decode_model_generalization_rules(&bytes) {
        Ok(rules) => format!("rules {}", rules.len()),
        Err(error) => error.to_string(),
    };
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
                    let (installed, _warnings) = CatalogGeneralizations::install(rules);
                    assert_eq!(
                        installed.match_routing("claude-x-1"),
                        installed.match_routing("CLAUDE-x-1")
                    );
                    drop(installed.match_capabilities(&name));
                    drop(installed.match_fill_missing(&name, "anthropic"));
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
