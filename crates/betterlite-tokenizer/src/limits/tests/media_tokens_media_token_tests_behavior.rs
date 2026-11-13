use betterlite_models::catalog::BundledCatalog;
use rstest::rstest;

use crate::limits::{Models, Modified, Settings, Warning, get_modified_max_tokens};

const GPT4O: &str = "gpt-4o";
const GPT35: &str = "gpt-3.5-turbo";
const GPT35_PREFIXED: &str = "openai/gpt-3.5-turbo";
const SHARED_CONTEXT: &str = "ai21.j2-mid-v1";
const NO_OUTPUT_LIMIT: &str = "databricks/databricks-grok-4-6";
const NO_INPUT_LIMIT: &str = "gemini/gemini-gemma-2-27b-it";
const UNKNOWN: &str = "not-a-model";
const HUGGINGFACE_BASE: &str = "huggingface/betterlite-test/not-in-the-catalog";
const UNKNOWN_PROVIDER: &str = "foo/gpt-3.5-turbo";
const NESTED_UNKNOWN_PROVIDER: &str = "a/b/gpt-3.5-turbo";

fn resolved(max_tokens: i64) -> Modified {
    Modified {
        max_tokens: Some(max_tokens),
        warnings: Vec::new(),
    }
}

fn absent() -> Modified {
    Modified {
        max_tokens: None,
        warnings: Vec::new(),
    }
}

fn warned(max_tokens: i64, warning: Warning) -> Modified {
    Modified {
        max_tokens: Some(max_tokens),
        warnings: vec![warning],
    }
}

fn buffer(buffer_fraction: f64, buffer_minimum_tokens: f64) -> Settings {
    Settings {
        buffer_fraction,
        buffer_minimum_tokens,
    }
}

#[rstest]
#[case::without_a_user_limit((GPT4O, GPT4O), None, Some(100), Settings::default(), absent())]
#[case::shared_context_trims_the_limit((SHARED_CONTEXT, SHARED_CONTEXT), Some(8000), Some(1000), Settings::default(), resolved(7091))]
#[case::shared_context_leaves_a_failing_call((SHARED_CONTEXT, SHARED_CONTEXT), Some(8000), Some(9000), Settings::default(), resolved(8000))]
#[case::shared_context_keeps_a_fitting_limit((SHARED_CONTEXT, SHARED_CONTEXT), Some(1000), Some(100), Settings::default(), resolved(1000))]
#[case::shared_context_boundary_not_greater((SHARED_CONTEXT, SHARED_CONTEXT), Some(8181), Some(0), Settings::default(), resolved(8181))]
#[case::shared_context_boundary_just_over((SHARED_CONTEXT, SHARED_CONTEXT), Some(8182), Some(0), Settings::default(), resolved(8181))]
#[case::shared_context_buffer_at_the_limit((SHARED_CONTEXT, SHARED_CONTEXT), Some(100), Some(8191), buffer(0.0, 0.0), resolved(0))]
#[case::separate_output_limit_clamps((GPT35, GPT35), Some(5000), Some(0), Settings::default(), resolved(4096))]
#[case::separate_output_limit_at_the_limit((GPT35, GPT35), Some(4096), Some(0), Settings::default(), resolved(4096))]
#[case::separate_output_limit_with_large_input((GPT35, GPT35), Some(5000), Some(5000), Settings::default(), resolved(4096))]
#[case::separate_output_limit_keeps_a_fitting_limit((GPT4O, GPT4O), Some(1000), Some(0), Settings::default(), resolved(1000))]
#[case::provider_prefixed_model_clamps((GPT35_PREFIXED, GPT35_PREFIXED), Some(5000), Some(0), Settings::default(), resolved(4096))]
#[case::unknown_provider_prefix_keeps_the_limit((UNKNOWN_PROVIDER, UNKNOWN_PROVIDER), Some(5000), Some(0), Settings::default(), warned(5000, Warning::UnknownModel { model: UNKNOWN_PROVIDER.to_owned() }))]
#[case::nested_unknown_provider_prefix_keeps_the_limit((NESTED_UNKNOWN_PROVIDER, NESTED_UNKNOWN_PROVIDER), Some(5000), Some(0), Settings::default(), warned(5000, Warning::UnknownModel { model: NESTED_UNKNOWN_PROVIDER.to_owned() }))]
#[case::huggingface_base_model_is_never_fetched((GPT4O, HUGGINGFACE_BASE), Some(5000), Some(0), Settings::default(), warned(5000, Warning::UnknownMaxOutputTokens { base_model: HUGGINGFACE_BASE.to_owned() }))]
#[case::default_percentage_boundary((SHARED_CONTEXT, SHARED_CONTEXT), Some(8100), Some(100), buffer(0.29, 10.0), resolved(8063))]
#[case::buffer_minimum_dominates((SHARED_CONTEXT, SHARED_CONTEXT), Some(8000), Some(1000), buffer(0.001, 50.0), resolved(7141))]
#[case::buffer_minimum_truncates((SHARED_CONTEXT, SHARED_CONTEXT), Some(8180), Some(10), buffer(0.1, 10.9), resolved(8171))]
#[case::negative_buffer_truncates_toward_zero((SHARED_CONTEXT, SHARED_CONTEXT), Some(8185), Some(100), buffer(-0.1, -20.0), resolved(8101))]
#[case::huge_input_tokens((SHARED_CONTEXT, SHARED_CONTEXT), Some(8000), Some(1_000_000_000_000), Settings::default(), resolved(8000))]
#[case::zero_user_limit((SHARED_CONTEXT, SHARED_CONTEXT), Some(0), Some(100), Settings::default(), resolved(0))]
#[case::negative_user_limit((SHARED_CONTEXT, SHARED_CONTEXT), Some(-5), Some(100), Settings::default(), resolved(-5))]
#[case::unknown_model_falls_back((UNKNOWN, UNKNOWN), Some(500), Some(0), Settings::default(), warned(500, Warning::UnknownModel { model: UNKNOWN.to_owned() }))]
#[case::unknown_base_model_falls_back((GPT4O, UNKNOWN), Some(500), Some(0), Settings::default(), warned(500, Warning::UnknownMaxOutputTokens { base_model: UNKNOWN.to_owned() }))]
#[case::model_without_an_output_limit_falls_back((GPT4O, NO_OUTPUT_LIMIT), Some(500), Some(0), Settings::default(), warned(500, Warning::UnknownMaxOutputTokens { base_model: NO_OUTPUT_LIMIT.to_owned() }))]
#[case::missing_max_input_tokens_clamps((NO_INPUT_LIMIT, NO_INPUT_LIMIT), Some(9000), Some(0), Settings::default(), resolved(8192))]
#[case::missing_max_input_tokens_keeps((NO_INPUT_LIMIT, NO_INPUT_LIMIT), Some(1000), Some(0), Settings::default(), resolved(1000))]
#[case::uncounted_input_falls_back((GPT35, GPT35), Some(5000), None, Settings::default(), warned(5000, Warning::InputNotCounted))]
#[case::nan_percentage_falls_back((SHARED_CONTEXT, SHARED_CONTEXT), Some(500), Some(100), buffer(f64::NAN, 10.0), warned(500, Warning::BufferNotFinite))]
#[case::nan_buffer_minimum_is_ignored((SHARED_CONTEXT, SHARED_CONTEXT), Some(8100), Some(100), buffer(0.1, f64::NAN), resolved(8081))]
#[case::infinite_percentage_falls_back((SHARED_CONTEXT, SHARED_CONTEXT), Some(500), Some(100), buffer(f64::INFINITY, 10.0), warned(500, Warning::BufferNotFinite))]
#[case::huge_buffer_minimum_stays_positive((SHARED_CONTEXT, SHARED_CONTEXT), Some(8000), Some(100), buffer(0.1, 1e300), resolved(8000))]
fn media_tokens_media_token_tests_behavior(
    #[case] models: (&str, &str),
    #[case] user_max_tokens: Option<i64>,
    #[case] input_tokens: Option<usize>,
    #[case] settings: Settings,
    #[case] expected: Modified,
) {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let modified = get_modified_max_tokens(
        &catalog,
        Models {
            model: models.0,
            base_model: models.1,
        },
        user_max_tokens,
        input_tokens,
        &settings,
    );
    assert_eq!(modified, expected);
}
