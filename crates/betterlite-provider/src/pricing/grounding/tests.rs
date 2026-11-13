#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::server_tool_projection::ServerToolUse;
use betterlite_models::cost_usage::tool_projection::ToolUsage;
use betterlite_models::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPricePolicy as _;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{GroundingFormulas, GroundingPricing, ProviderFormulas, SearchFormula};

struct Recording {
    web: Mutex<Vec<SearchFormula>>,
    maps: Mutex<Vec<i64>>,
}

impl Recording {
    fn new() -> Self {
        Self {
            web: Mutex::new(Vec::new()),
            maps: Mutex::new(Vec::new()),
        }
    }
}

impl GroundingFormulas for &Recording {
    fn web_search(
        &self,
        formula: SearchFormula,
        _usage: &ToolUsage,
        _model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        self.web.lock().unwrap().push(formula);
        Some(Decimal::new(5, 0))
    }

    fn google_maps(&self, requests: i64, _model_info: &ToolModelInfo) -> Option<Decimal> {
        self.maps.lock().unwrap().push(requests);
        Some(Decimal::new(9, 0))
    }
}

fn info(medium: Rate) -> ToolModelInfo {
    ToolModelInfo {
        litellm_provider: None,
        search_context: SearchContextRates {
            low: Rate::Absent,
            medium,
            high: Rate::Absent,
        },
        file_search_per_gb_per_day: Rate::Absent,
        file_search_per_1k_calls: Rate::Absent,
        vector_store_per_gb_per_day: Rate::Absent,
        computer_input_per_1k: Rate::Absent,
        computer_output_per_1k: Rate::Absent,
        code_interpreter_per_session: Rate::Absent,
    }
}

fn usage(web_search_requests: Option<i64>) -> ToolUsage {
    ToolUsage {
        cost: CostUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_details: None,
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        server_tool_use: web_search_requests.map(|count| ServerToolUse {
            web_search_requests: Some(count),
            tool_search_requests: None,
            browser_open_requests: None,
        }),
        server_side_details: None,
    }
}

#[rstest]
#[case::gemini("gemini", None, Some(SearchFormula::Gemini))]
#[case::anthropic("anthropic", None, Some(SearchFormula::Anthropic))]
#[case::xai("xai", None, Some(SearchFormula::Xai))]
#[case::groq("groq", None, Some(SearchFormula::Groq))]
#[case::vertex_gemini("vertex_ai", Some("gemini-2.5-pro"), Some(SearchFormula::VertexGemini))]
#[case::vertex_without_key("vertex_ai", None, Some(SearchFormula::VertexGemini))]
#[case::vertex_claude("vertex_ai", Some("claude-sonnet-4"), Some(SearchFormula::Anthropic))]
#[case::vertex_claude_upper("vertex_ai", Some("CLAUDE-3"), Some(SearchFormula::Anthropic))]
#[case::vertex_claude_inside(
    "vertex_ai",
    Some("vertex_ai/claude-3"),
    Some(SearchFormula::Anthropic)
)]
#[case::vertex_beta_prefix("vertex_ai-anthropic", None, Some(SearchFormula::VertexGemini))]
#[case::vertex_beta_claude("vertex_ai_beta", Some("claude"), Some(SearchFormula::Anthropic))]
#[case::gemini_model_key_is_ignored("gemini", Some("claude"), Some(SearchFormula::Gemini))]
#[case::unknown("openai", None, None)]
#[case::empty("", None, None)]
#[case::case_sensitive("Gemini", None, None)]
#[case::bedrock("bedrock", None, None)]
fn web_search_routes_by_provider_name(
    #[case] provider: &str,
    #[case] model_key: Option<&str>,
    #[case] expected: Option<SearchFormula>,
) {
    let recording = Recording::new();
    let policy = GroundingPricing {
        formulas: &recording,
        model_key,
    };
    let cost = policy.web_search_cost(provider, &usage(Some(1)), &info(Rate::Absent));
    assert_eq!(
        *recording.web.lock().unwrap(),
        expected.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(cost, expected.map(|_| Decimal::new(5, 0)));
}

#[test]
fn perplexity_is_already_billed_and_reaches_no_formula() {
    let recording = Recording::new();
    let policy = GroundingPricing {
        formulas: &recording,
        model_key: None,
    };
    let cost = policy.web_search_cost("perplexity", &usage(Some(4)), &info(Rate::Absent));
    assert_eq!(cost, Some(Decimal::ZERO));
    assert!(recording.web.lock().unwrap().is_empty());
}

#[rstest]
#[case::gemini("gemini", true)]
#[case::vertex("vertex_ai", true)]
#[case::vertex_beta("vertex_ai_beta", true)]
#[case::vertex_dashed("vertex_ai-language-models", true)]
#[case::anthropic("anthropic", false)]
#[case::openai("openai", false)]
#[case::empty("", false)]
#[case::upper("VERTEX_AI", false)]
#[case::prefixed("my_vertex_ai", false)]
fn google_maps_is_priced_only_for_gemini_and_vertex(#[case] provider: &str, #[case] priced: bool) {
    let recording = Recording::new();
    let policy = GroundingPricing {
        formulas: &recording,
        model_key: None,
    };
    let cost = policy.google_maps_cost(provider, 3, &info(Rate::Absent));
    assert_eq!(cost, priced.then(|| Decimal::new(9, 0)));
    assert_eq!(
        *recording.maps.lock().unwrap(),
        if priced { vec![3] } else { Vec::new() }
    );
}

#[rstest]
#[case::rate_times_count(Rate::Value(Decimal::new(1, 2)), Some(3), Some(Decimal::new(3, 2)))]
#[case::zero_rate(Rate::Value(Decimal::ZERO), Some(3), Some(Decimal::ZERO))]
#[case::absent_rate(Rate::Absent, Some(3), Some(Decimal::ZERO))]
#[case::null_rate(Rate::Null, Some(3), Some(Decimal::ZERO))]
#[case::no_count(Rate::Value(Decimal::new(1, 2)), None, Some(Decimal::ZERO))]
#[case::zero_count(Rate::Value(Decimal::new(1, 2)), Some(0), Some(Decimal::ZERO))]
fn anthropic_search_is_the_medium_rate_times_the_reported_count(
    #[case] medium: Rate,
    #[case] count: Option<i64>,
    #[case] expected: Option<Decimal>,
) {
    let policy = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: None,
    };
    assert_eq!(
        policy.web_search_cost("anthropic", &usage(count), &info(medium)),
        expected
    );
}

#[test]
fn a_vertex_claude_model_is_priced_by_the_anthropic_formula() {
    let policy = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: Some("vertex_ai/claude-3-5"),
    };
    let rate = Rate::Value(Decimal::new(1, 2));
    assert_eq!(
        policy.web_search_cost("vertex_ai", &usage(Some(2)), &info(rate)),
        Some(Decimal::new(2, 2))
    );
}

#[rstest]
#[case::gemini("gemini")]
#[case::vertex_gemini("vertex_ai")]
#[case::xai("xai")]
#[case::groq("groq")]
fn formulas_without_an_installed_owner_have_no_price(#[case] provider: &str) {
    let policy = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: None,
    };
    let rate = Rate::Value(Decimal::new(1, 2));
    assert_eq!(
        policy.web_search_cost(provider, &usage(Some(2)), &info(rate)),
        None
    );
}

#[test]
fn google_maps_has_no_installed_formula() {
    let policy = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: None,
    };
    assert_eq!(
        policy.google_maps_cost("gemini", 2, &info(Rate::Absent)),
        None
    );
}

#[test]
fn an_anthropic_arithmetic_failure_yields_no_price() {
    let policy = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: None,
    };
    assert_eq!(
        policy.web_search_cost(
            "anthropic",
            &usage(Some(i64::MAX)),
            &info(Rate::Value(Decimal::MAX))
        ),
        None
    );
}
