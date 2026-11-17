#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_models::catalog_generalizations::ModelInfo;
use betterlite_models::error::ModelsError;
use rstest::rstest;
use serde::Serialize;

use super::{Found, ModelGeneralizations, fill_absent};
use crate::generalization_integration::bounds::MAX_RULES_BYTES;
use crate::generalization_integration::error::GeneralizationError;
use crate::tag_matchers::patterns::MAX_PATTERN_BYTES;

const FIRST_ROUTING: &str = r#"[{"pattern":"^acme-","model_info":{"litellm_provider":"acme"}},{"pattern":"acme-fast","model_info":{"litellm_provider":"later"}}]"#;
const CASE_INSENSITIVE: &str =
    r#"[{"pattern":"^CLAUDE-","model_info":{"litellm_provider":"anthropic"}}]"#;
const EXACT_RULE: &str = r#"[{"pattern":"^gpt-","model_info":{"litellm_provider":"wrong"}}]"#;
const RELOAD_OLD: &str = r#"[{"pattern":"^old-","model_info":{"litellm_provider":"first"}}]"#;
const RELOAD_NEW: &str = r#"[{"pattern":"^new-","model_info":{"litellm_provider":"second"}}]"#;
const CAPABILITIES: &str = r#"[{"pattern":"^gemini","model_info":{"supports_vision":true,"max_tokens":100}},{"pattern":"gemini-3","model_info":{"supports_vision":false,"max_tokens":null}},{"pattern":"^empty$","model_info":{}}]"#;
const FILL_MISSING: &str = r#"[{"pattern":"gemini","model_info":{"litellm_provider":"vertex_ai","supports_pdf":true},"fill_missing_for_providers":["vertex_ai"]},{"pattern":"gemini-3","model_info":{"supports_pdf":false,"max_tokens":null},"fill_missing_for_providers":["vertex_ai","gemini"]},{"pattern":"^route$","model_info":{"litellm_provider":"openai"},"fill_missing_for_providers":["openai"]},{"pattern":"^plain$","model_info":{"mode":"chat"}}]"#;
const LEGACY_PARENT: &str = r#"[{"name":"base","pattern":"^base-","model_info":{"mode":"chat","max_tokens":1}},{"name":"child","pattern":"^child-","extends":"base","model_info":{"max_tokens":9}},{"name":"grand","pattern":"^grand-","extends":"child","model_info":{"x":1}}]"#;
const DUPLICATE_PARENT: &str = r#"[{"name":"base","pattern":"^first-","model_info":{"mode":"chat"}},{"name":"base","pattern":"^second-","model_info":{"mode":"embedding"}},{"name":"child","pattern":"^child-","extends":"base","model_info":{"x":1}}]"#;
const DESCRIPTION: &str = r#"[{"pattern":"^acme-","description":"ignored text","model_info":{"litellm_provider":"acme","mode":"chat"}}]"#;
const MALFORMED: &str = r#"[{"name":"broken","pattern":"^broken"},{"pattern":"^ok-","model_info":{"litellm_provider":"openai"}}]"#;
const MALFORMED_WARNING: &str = r#"["LiteLLM: skipping malformed fallback generalization rule broken (needs string 'pattern' and dict 'model_info')."]"#;
const SLOW_PATTERN: &str =
    r#"[{"pattern":"(?i)(a|b|ab)*(?>c)","model_info":{"litellm_provider":"steps"}}]"#;
const SLOW_MODEL: &str = "abababababababababababab";
const SLOW_WARNING: &str = r#"[{"kind":"match_limit_exceeded","pattern":"(?i)(a|b|ab)*(?>c)"}]"#;

#[derive(Debug, Clone, Copy)]
enum Probe {
    Routing(&'static str),
    Capabilities(&'static str),
    FillMissing(&'static str, &'static str),
    Provider(&'static str, Option<&'static str>),
    Reload(&'static str),
    Warnings,
    EngineWarnings,
    FillAbsent(&'static str, &'static str),
}

fn install(rules: &str) -> ModelGeneralizations {
    ModelGeneralizations::install(rules.as_bytes()).unwrap()
}

fn info(source: &str) -> ModelInfo {
    serde_json::from_str(source).unwrap()
}

fn found_text<T>(found: &Found<T>) -> String
where
    T: Serialize,
{
    let answer = serde_json::to_string(&found.answer).unwrap();
    let warnings = serde_json::to_string(&found.warnings).unwrap();
    format!("{answer} {warnings}")
}

fn json_text<T>(value: &T) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).unwrap()
}

fn rendered(service: &mut ModelGeneralizations, probe: Probe) -> String {
    match probe {
        Probe::Routing(model) => {
            format!("route {model} {}", found_text(&service.routing(model)))
        }
        Probe::Capabilities(model) => {
            format!(
                "capabilities {model} {}",
                found_text(&service.capabilities(model))
            )
        }
        Probe::FillMissing(model, provider) => {
            let found = service.fill_missing(model, provider);
            format!("fill {model}/{provider} {}", found_text(&found))
        }
        Probe::Provider(model, exact) => {
            let found = service.provider(model, exact);
            format!("provider {model} {}", found_text(&found))
        }
        Probe::Reload(rules) => match service.reload(rules.as_bytes()) {
            Ok(()) => String::from("reload ok"),
            Err(GeneralizationError::Rules(ModelsError::RulesJson { .. })) => {
                String::from("reload rules-json")
            }
            Err(GeneralizationError::Rules(_)) => String::from("reload other-rules"),
            Err(GeneralizationError::Json { .. }) => String::from("reload json"),
        },
        Probe::Warnings => format!("warnings {}", json_text(&service.warnings)),
        Probe::EngineWarnings => format!("engine {}", json_text(&service.engine_warnings)),
        Probe::FillAbsent(exact, proposed) => {
            let mut exact_info = info(exact);
            fill_absent(&mut exact_info, info(proposed));
            format!("fill-absent {}", json_text(&exact_info))
        }
    }
}

#[rstest]
#[case::first_routing_match(
    FIRST_ROUTING,
    &[Probe::Routing("acme-fast"), Probe::Routing("acme-slow"), Probe::Routing("other")],
    &[r#"route acme-fast "acme" []"#, r#"route acme-slow "acme" []"#, "route other null []"],
)]
#[case::case_insensitive_search(
    CASE_INSENSITIVE,
    &[Probe::Routing("claude-3-5-sonnet"), Probe::Routing("Claude-3")],
    &[r#"route claude-3-5-sonnet "anthropic" []"#, r#"route Claude-3 "anthropic" []"#],
)]
#[case::exact_entry_before_rule(
    EXACT_RULE,
    &[
        Probe::Provider("gpt-4o", Some("openai")),
        Probe::Provider("gpt-4o", None),
        Probe::Provider("gpt-4o", Some("")),
        Probe::Routing("gpt-4o"),
    ],
    &[
        r#"provider gpt-4o "openai" []"#,
        r#"provider gpt-4o "wrong" []"#,
        r#"provider gpt-4o "wrong" []"#,
        r#"route gpt-4o "wrong" []"#,
    ],
)]
#[case::reload_replaces_rules(
    RELOAD_OLD,
    &[
        Probe::Routing("old-model"),
        Probe::Reload(RELOAD_NEW),
        Probe::Routing("old-model"),
        Probe::Routing("new-model"),
        Probe::Reload("{"),
        Probe::Routing("new-model"),
    ],
    &[
        r#"route old-model "first" []"#,
        "reload ok",
        "route old-model null []",
        r#"route new-model "second" []"#,
        "reload rules-json",
        r#"route new-model "second" []"#,
    ],
)]
#[case::explicit_null_not_filled(
    "[]",
    &[
        Probe::FillAbsent(
            r#"{"max_tokens":null,"supports_vision":false}"#,
            r#"{"max_tokens":4096,"supports_vision":true,"mode":"chat"}"#,
        ),
        Probe::FillAbsent("{}", r#"{"max_tokens":null,"mode":"chat"}"#),
    ],
    &[
        r#"fill-absent {"max_tokens":null,"mode":"chat","supports_vision":false}"#,
        r#"fill-absent {"max_tokens":null,"mode":"chat"}"#,
    ],
)]
#[case::capability_union_last_wins(
    CAPABILITIES,
    &[
        Probe::Capabilities("gemini-3-pro"),
        Probe::Capabilities("gemini"),
        Probe::Capabilities("empty"),
        Probe::Capabilities("gpt"),
        Probe::Capabilities(""),
    ],
    &[
        r#"capabilities gemini-3-pro {"max_tokens":null,"supports_vision":false} []"#,
        r#"capabilities gemini {"max_tokens":100,"supports_vision":true} []"#,
        "capabilities empty {} []",
        "capabilities gpt null []",
        "capabilities  null []",
    ],
)]
#[case::fill_missing_provider_filter(
    FILL_MISSING,
    &[
        Probe::FillMissing("gemini-3-pro", "vertex_ai"),
        Probe::FillMissing("gemini-3", "gemini"),
        Probe::FillMissing("gemini", "Vertex_AI"),
        Probe::FillMissing("gemini-3", ""),
        Probe::FillMissing("", "vertex_ai"),
        Probe::FillMissing("route", "openai"),
        Probe::FillMissing("plain", "openai"),
    ],
    &[
        r#"fill gemini-3-pro/vertex_ai {"max_tokens":null,"supports_pdf":false} []"#,
        r#"fill gemini-3/gemini {"max_tokens":null,"supports_pdf":false} []"#,
        "fill gemini/Vertex_AI null []",
        "fill gemini-3/ null []",
        "fill /vertex_ai null []",
        "fill route/openai null []",
        "fill plain/openai null []",
    ],
)]
#[case::legacy_single_level_parent(
    LEGACY_PARENT,
    &[
        Probe::Capabilities("child-x"),
        Probe::Capabilities("grand-x"),
        Probe::Capabilities("base-x"),
    ],
    &[
        r#"capabilities child-x {"max_tokens":9,"mode":"chat"} []"#,
        r#"capabilities grand-x {"max_tokens":9,"x":1} []"#,
        r#"capabilities base-x {"max_tokens":1,"mode":"chat"} []"#,
    ],
)]
#[case::duplicate_name_parent_last_wins(
    DUPLICATE_PARENT,
    &[
        Probe::Capabilities("child-x"),
        Probe::Capabilities("first-x"),
        Probe::Capabilities("second-x"),
    ],
    &[
        r#"capabilities child-x {"mode":"embedding","x":1} []"#,
        r#"capabilities first-x {"mode":"chat"} []"#,
        r#"capabilities second-x {"mode":"embedding"} []"#,
    ],
)]
#[case::unknown_description_ignored(
    DESCRIPTION,
    &[
        Probe::Routing("acme-1"),
        Probe::Capabilities("acme-1"),
        Probe::Warnings,
        Probe::EngineWarnings,
    ],
    &[
        r#"route acme-1 "acme" []"#,
        r#"capabilities acme-1 {"litellm_provider":"acme","mode":"chat"} []"#,
        "warnings []",
        "engine []",
    ],
)]
#[case::malformed_rule_warning_skip(
    MALFORMED,
    &[
        Probe::EngineWarnings,
        Probe::Warnings,
        Probe::Routing("ok-1"),
        Probe::Routing("broken-1"),
    ],
    &[
        MALFORMED_WARNING,
        "warnings []",
        r#"route ok-1 "openai" []"#,
        "route broken-1 null []",
    ],
)]
fn model_generalizations_behaviors(
    #[case] rules: &str,
    #[case] probes: &[Probe],
    #[case] expected: &[&str],
) {
    let mut service = install(rules);
    let lines: Vec<String> = probes
        .iter()
        .map(|probe| rendered(&mut service, *probe))
        .collect();
    assert_eq!(lines, expected);
}

#[derive(Debug, Clone, Copy)]
enum Spec {
    EmptyRules,
    NonListDocument,
    RulesAtLimit,
    RulesOverLimit,
    InvalidDocument,
    PatternAtLimit,
    PatternOverLimit,
    SlowPattern,
    SlowCapabilityRules,
    LargeAndSlowPattern,
}

#[derive(Debug, Clone, Copy)]
enum Boundary {
    Outcome,
    Warnings,
    Routing(&'static str),
    Capabilities(&'static str),
    FillMissing(&'static str, &'static str),
}

fn padded(size: usize) -> Vec<u8> {
    let mut document = vec![b' '; size];
    document[..2].copy_from_slice(b"[]");
    document
}

fn large_pattern_rule() -> String {
    let pattern = "a".repeat(MAX_PATTERN_BYTES.saturating_add(1));
    format!(r#"{{"pattern":"{pattern}","model_info":{{"litellm_provider":"big"}}}}"#)
}

const fn slow_pattern_rule() -> &'static str {
    r#"{"pattern":"(?i)(a|b|ab)*(?>c)","model_info":{"litellm_provider":"steps"}}"#
}

const SLOW_CAPABILITY_RULES: &str = concat!(
    r#"[{"pattern":"(?i)(a|b|ab)*(?>c)","model_info":{"supports_vision":true}},"#,
    r#"{"pattern":"(?i)(a|b|ab)*(?>c)","model_info":{"mode":"chat"},"fill_missing_for_providers":["acme"]},"#,
    r#"{"pattern":"(?i)(a|b|ab)*(?>c)","model_info":{"mode":"embedding"},"fill_missing_for_providers":[""]}]"#,
);

fn rules(spec: Spec) -> Vec<u8> {
    match spec {
        Spec::EmptyRules => b"[]".to_vec(),
        Spec::NonListDocument => br#"{"rules":[]}"#.to_vec(),
        Spec::RulesAtLimit => padded(MAX_RULES_BYTES),
        Spec::RulesOverLimit => padded(MAX_RULES_BYTES.saturating_add(1)),
        Spec::InvalidDocument => b"{".to_vec(),
        Spec::PatternAtLimit => {
            let literal = "a".repeat(MAX_PATTERN_BYTES.saturating_sub(4));
            format!(r#"[{{"pattern":"{literal}|zzz","model_info":{{"litellm_provider":"big"}}}}]"#)
                .into_bytes()
        }
        Spec::PatternOverLimit => format!("[{}]", large_pattern_rule()).into_bytes(),
        Spec::SlowPattern => format!("[{}]", slow_pattern_rule()).into_bytes(),
        Spec::SlowCapabilityRules => SLOW_CAPABILITY_RULES.as_bytes().to_vec(),
        Spec::LargeAndSlowPattern => {
            format!("[{},{}]", large_pattern_rule(), slow_pattern_rule()).into_bytes()
        }
    }
}

fn large_pattern_warning() -> String {
    let pattern = "a".repeat(MAX_PATTERN_BYTES.saturating_add(1));
    format!(r#"[{{"kind":"pattern_too_large","pattern":"{pattern}"}}]"#)
}

fn slow_warning() -> String {
    String::from(r#"{"kind":"match_limit_exceeded","pattern":"(?i)(a|b|ab)*(?>c)"}"#)
}

fn install_error(error: &GeneralizationError) -> String {
    match error {
        GeneralizationError::Rules(ModelsError::RulesTooLarge { size }) => {
            format!("install too-large {size}")
        }
        GeneralizationError::Rules(ModelsError::RulesJson { .. }) => {
            String::from("install rules-json")
        }
        GeneralizationError::Rules(_) => String::from("install other-rules"),
        GeneralizationError::Json { .. } => String::from("install json"),
    }
}

fn boundary_line(
    installed: &Result<ModelGeneralizations, GeneralizationError>,
    probe: Boundary,
) -> String {
    match installed {
        Err(error) => match probe {
            Boundary::Outcome => install_error(error),
            Boundary::Warnings => String::from("warnings none"),
            Boundary::Routing(_) => String::from("route not-installed"),
            Boundary::Capabilities(_) => String::from("capabilities not-installed"),
            Boundary::FillMissing(..) => String::from("fill not-installed"),
        },
        Ok(service) => match probe {
            Boundary::Outcome => String::from("install ok"),
            Boundary::Warnings => format!("warnings {}", json_text(&service.warnings)),
            Boundary::Routing(model) => {
                format!("route {model} {}", found_text(&service.routing(model)))
            }
            Boundary::Capabilities(model) => {
                format!(
                    "capabilities {model} {}",
                    found_text(&service.capabilities(model))
                )
            }
            Boundary::FillMissing(model, provider) => {
                let found = service.fill_missing(model, provider);
                format!("fill {model}/{provider} {}", found_text(&found))
            }
        },
    }
}

#[rstest]
#[case::empty_document(
    Spec::EmptyRules,
    &[Boundary::Outcome, Boundary::Warnings],
    &["install ok", "warnings []"],
)]
#[case::non_list_document(
    Spec::NonListDocument,
    &[Boundary::Outcome, Boundary::Warnings],
    &["install ok", "warnings []"],
)]
#[case::rules_at_byte_limit(
    Spec::RulesAtLimit,
    &[Boundary::Outcome, Boundary::Warnings],
    &["install ok", "warnings []"],
)]
#[case::rules_over_byte_limit(
    Spec::RulesOverLimit,
    &[Boundary::Outcome],
    &["install too-large 1048577"],
)]
#[case::invalid_document(
    Spec::InvalidDocument,
    &[Boundary::Outcome],
    &["install rules-json"],
)]
#[case::pattern_at_byte_limit(
    Spec::PatternAtLimit,
    &[Boundary::Outcome, Boundary::Warnings, Boundary::Routing("zzz")],
    &["install ok", "warnings []", r#"route zzz "big" []"#],
)]
#[case::pattern_over_byte_limit(
    Spec::PatternOverLimit,
    &[Boundary::Outcome, Boundary::Warnings, Boundary::Routing("zzz"), Boundary::Routing("aaa")],
    &["install ok", &large_pattern_warning(), "route zzz null []", "route aaa null []"],
)]
#[case::match_steps_within_limit(
    Spec::SlowPattern,
    &[Boundary::Routing("abab")],
    &["route abab null []"],
)]
#[case::match_steps_over_limit(
    Spec::SlowPattern,
    &[Boundary::Routing(SLOW_MODEL)],
    &[&format!("route {SLOW_MODEL} null {SLOW_WARNING}")],
)]
#[case::capability_match_steps_over_limit(
    Spec::SlowCapabilityRules,
    &[
        Boundary::Capabilities(SLOW_MODEL),
        Boundary::Capabilities("abab"),
        Boundary::Warnings,
    ],
    &[
        &format!(
            "capabilities {SLOW_MODEL} null [{},{},{}]",
            slow_warning(),
            slow_warning(),
            slow_warning()
        ),
        "capabilities abab null []",
        "warnings []",
    ],
)]
#[case::fill_missing_match_steps_over_limit(
    Spec::SlowCapabilityRules,
    &[
        Boundary::FillMissing(SLOW_MODEL, "acme"),
        Boundary::FillMissing(SLOW_MODEL, ""),
    ],
    &[
        &format!("fill {SLOW_MODEL}/acme null [{}]", slow_warning()),
        &format!("fill {SLOW_MODEL}/ null []"),
    ],
)]
#[case::large_and_slow_together(
    Spec::LargeAndSlowPattern,
    &[Boundary::Warnings, Boundary::Routing(SLOW_MODEL)],
    &[&large_pattern_warning(), &format!("route {SLOW_MODEL} null {SLOW_WARNING}")],
)]
fn model_generalizations_boundaries(
    #[case] spec: Spec,
    #[case] probes: &[Boundary],
    #[case] expected: &[&str],
) {
    let installed = ModelGeneralizations::install(&rules(spec));
    let mut lines = Vec::new();
    for probe in probes {
        lines.push(boundary_line(&installed, *probe));
    }
    assert_eq!(lines, expected);
}

#[test]
fn replays_the_model_rules_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/model_rules");
    let valid = fs::read(corpus.join("model-generalizations-valid.json")).unwrap();
    let invalid = fs::read(corpus.join("model-generalizations-invalid.json")).unwrap();
    let installed = ModelGeneralizations::install(&valid).unwrap();
    assert_eq!(
        installed.routing("corpus-model").answer.as_deref(),
        Some("corpus")
    );
    assert!(installed.warnings.is_empty());
    assert!(installed.engine_warnings.is_empty());
    drop(installed.capabilities("corpus-model"));
    drop(installed.fill_missing("corpus-model", "corpus"));
    assert!(matches!(
        ModelGeneralizations::install(&invalid),
        Err(GeneralizationError::Rules(ModelsError::RulesJson { .. }))
    ));
    fs::read_dir(corpus).unwrap().for_each(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        drop(ModelGeneralizations::install(&bytes));
    });
}
