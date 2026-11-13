#![cfg(test)]

use std::mem;
use std::sync::{Mutex, PoisonError};

use betterlite_models::cost_usage::tool_projection::ToolUsage;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPricePolicy;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::error::ImageDispatchError;
use super::grounding::image_grounding_cost;
use super::options::{
    OptionalParams, SizePattern, call_type_has_image_response, requested_image_param,
};
use super::policy::{ImagePricePolicy, ImageResponseView, ImageRoute};
use super::route::{ImageCall, route_image_cost};

#[derive(Debug, thiserror::Error)]
#[error("the recording policy refused")]
struct Refused;

#[derive(Debug)]
struct Seen {
    label: &'static str,
    size: Option<String>,
    quality: Option<String>,
    image_count: Option<i64>,
    provider: Option<String>,
    price_keys: Option<Vec<String>>,
    image_price: Option<Scalar>,
    has_params: bool,
}

struct Recording {
    seen: Mutex<Vec<Seen>>,
    refuse: bool,
}

impl Recording {
    fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            refuse: false,
        }
    }

    fn calls(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

impl Seen {
    fn new(label: &'static str, prices: Option<&PriceRecord>) -> Self {
        Self {
            label,
            size: None,
            quality: None,
            image_count: None,
            provider: None,
            price_keys: prices.map(|record| record.keys().cloned().collect()),
            image_price: prices.and_then(|record| record.get("output_cost_per_image").cloned()),
            has_params: false,
        }
    }
}

fn route_prices<'route>(route: &'route ImageRoute<'_>) -> Option<&'route PriceRecord> {
    match route {
        ImageRoute::Bedrock { .. } => None,
        ImageRoute::VertexAi(image)
        | ImageRoute::Recraft(image)
        | ImageRoute::Aiml(image)
        | ImageRoute::Cometapi(image)
        | ImageRoute::Gemini(image)
        | ImageRoute::GeminiEdit(image)
        | ImageRoute::Runwayml(image)
        | ImageRoute::AzureAi { image, .. }
        | ImageRoute::FalAi { image, .. }
        | ImageRoute::GptImage { image, .. }
        | ImageRoute::Default { image, .. } => image.prices,
    }
}

fn describe(route: &ImageRoute<'_>) -> Seen {
    let label = if matches!(route, ImageRoute::GeminiEdit(_)) {
        "gemini_edit"
    } else {
        route.adapter()
    };
    let mut seen = Seen::new(label, route_prices(route));
    match route {
        ImageRoute::Bedrock {
            size,
            optional_params,
            ..
        }
        | ImageRoute::AzureAi {
            size,
            optional_params,
            ..
        }
        | ImageRoute::Default {
            size,
            optional_params,
            ..
        } => {
            seen.size = Some((*size).to_owned());
            seen.has_params = optional_params.is_some();
        }
        ImageRoute::FalAi {
            optional_params, ..
        } => seen.has_params = optional_params.is_some(),
        ImageRoute::VertexAi(_)
        | ImageRoute::Recraft(_)
        | ImageRoute::Aiml(_)
        | ImageRoute::Cometapi(_)
        | ImageRoute::Gemini(_)
        | ImageRoute::GeminiEdit(_)
        | ImageRoute::Runwayml(_)
        | ImageRoute::GptImage { .. } => {}
    }
    note_billing_fields(&mut seen, route);
    seen
}

fn note_billing_fields(seen: &mut Seen, route: &ImageRoute<'_>) {
    match route {
        ImageRoute::AzureAi { image_count, .. } => seen.image_count = Some(*image_count),
        ImageRoute::Default {
            provider,
            quality,
            image_count,
            ..
        } => {
            seen.quality = Some((*quality).to_owned());
            seen.image_count = Some(*image_count);
            seen.provider = provider.map(str::to_owned);
        }
        ImageRoute::GptImage { provider, .. } => seen.provider = Some((*provider).to_owned()),
        ImageRoute::Bedrock { .. }
        | ImageRoute::FalAi { .. }
        | ImageRoute::VertexAi(_)
        | ImageRoute::Recraft(_)
        | ImageRoute::Aiml(_)
        | ImageRoute::Cometapi(_)
        | ImageRoute::Gemini(_)
        | ImageRoute::GeminiEdit(_)
        | ImageRoute::Runwayml(_) => {}
    }
}

impl ImagePricePolicy for Recording {
    type Error = Refused;

    fn image_cost(&self, route: &ImageRoute<'_>) -> Result<Decimal, Refused> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(describe(route));
        if self.refuse {
            return Err(Refused);
        }
        Ok(Decimal::new(7, 1))
    }
}

fn params(entries: &[(&str, Json)]) -> OptionalParams {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn billing(count: i64, size: Option<&str>, quality: Option<&str>) -> ImageBilling {
    ImageBilling {
        image_count: count,
        size: size.map(str::to_owned),
        quality: quality.map(str::to_owned),
        usage: None,
    }
}

fn call<'call>(provider: Option<&'call str>, response: &'call ImageBilling) -> ImageCall<'call> {
    ImageCall {
        model: "image-model",
        custom_llm_provider: provider,
        quality: None,
        image_count: None,
        size: None,
        optional_params: None,
        call_type: None,
        model_info: None,
        response: ImageResponseView::Image(response),
    }
}

fn run(call: &ImageCall<'_>) -> (Result<Decimal, ImageDispatchError<Refused>>, Vec<Seen>) {
    let policy = Recording::new();
    let pattern = SizePattern::new().unwrap();
    let cost = route_image_cost(&policy, &pattern, call);
    (cost, policy.calls())
}

#[rstest]
#[case::lower_x("1024x1024", true)]
#[case::dashed("1024-x-1024", true)]
#[case::fullwidth_digits(
    "\u{ff11}\u{ff10}\u{ff12}\u{ff14}x\u{ff11}\u{ff10}\u{ff12}\u{ff14}",
    true
)]
#[case::arabic_indic_digits("\u{663}x\u{664}", true)]
#[case::leading_space(" 1x1", false)]
#[case::trailing_space("1x1 ", false)]
#[case::trailing_newline("1x1\n", false)]
#[case::plus_sign("+1x1", false)]
#[case::minus_sign("-1x1", false)]
#[case::negative_height("1x-1", false)]
#[case::auto("auto", false)]
#[case::capital_x("1X1", false)]
#[case::half_dashed("1-x1", false)]
#[case::missing_width("x1", false)]
#[case::missing_height("1x", false)]
#[case::three_parts("1x1x1", false)]
#[case::empty("", false)]
fn the_optional_size_must_fully_match_decimal_digits(#[case] size: &str, #[case] accepted: bool) {
    let options = params(&[("size", text(size))]);
    let pattern = SizePattern::new().unwrap();
    let expected = accepted.then_some(size);
    assert_eq!(pattern.requested_size(Some(&options)), expected);
}

#[rstest]
#[case::number(Json::Number(Number::from(1_i64)))]
#[case::null(Json::Null)]
#[case::boolean(Json::Bool(true))]
#[case::list(Json::List(vec![text("1x1")]))]
#[case::object(Json::Object(params(&[("size", text("1x1"))])))]
fn a_requested_field_that_is_not_a_string_is_absent(#[case] value: Json) {
    let options = params(&[("size", value.clone()), ("quality", value)]);
    let pattern = SizePattern::new().unwrap();
    assert_eq!(requested_image_param(Some(&options), "quality"), None);
    assert_eq!(pattern.requested_size(Some(&options)), None);
}

#[test]
fn a_missing_key_or_missing_options_is_absent() {
    let options = params(&[("quality", text("hd"))]);
    let pattern = SizePattern::new().unwrap();
    assert_eq!(requested_image_param(Some(&options), "quality"), Some("hd"));
    assert_eq!(requested_image_param(Some(&options), "style"), None);
    assert_eq!(requested_image_param(None, "quality"), None);
    assert_eq!(pattern.requested_size(None), None);
}

#[rstest]
#[case::generation("image_generation", true)]
#[case::async_generation("aimage_generation", true)]
#[case::passthrough("passthrough-image-generation", true)]
#[case::edit("image_edit", true)]
#[case::async_edit("aimage_edit", true)]
#[case::completion("completion", false)]
#[case::variation("image_variation", false)]
#[case::async_variation("aimage_variation", false)]
#[case::prefixed("pass_through_image_generation", false)]
#[case::underscored("passthrough_image_generation", false)]
#[case::upper("IMAGE_GENERATION", false)]
#[case::empty("", false)]
fn only_the_five_image_call_names_have_an_image_response(
    #[case] call_type: &str,
    #[case] expected: bool,
) {
    assert_eq!(call_type_has_image_response(call_type), expected);
}

#[test]
fn size_prefers_the_explicit_value_then_the_response_then_a_valid_option_then_the_default() {
    let options = params(&[("size", text("64x64"))]);
    let response = billing(1, Some("512x512"), None);
    let mut explicit = call(None, &response);
    explicit.size = Some("256x256");
    explicit.optional_params = Some(&options);
    assert_eq!(run(&explicit).1[0].size.as_deref(), Some("256x256"));

    let mut from_response = call(None, &response);
    from_response.optional_params = Some(&options);
    assert_eq!(run(&from_response).1[0].size.as_deref(), Some("512x512"));

    let bare = billing(1, None, None);
    let mut from_options = call(None, &bare);
    from_options.optional_params = Some(&options);
    assert_eq!(run(&from_options).1[0].size.as_deref(), Some("64x64"));

    let invalid = params(&[("size", text("auto"))]);
    let mut defaulted = call(None, &bare);
    defaulted.optional_params = Some(&invalid);
    assert_eq!(run(&defaulted).1[0].size.as_deref(), Some("1024-x-1024"));
}

#[test]
fn an_explicit_or_returned_size_bypasses_the_option_pattern() {
    let response = billing(1, Some("auto"), None);
    let mut explicit = call(None, &response);
    explicit.size = Some("not a size");
    assert_eq!(run(&explicit).1[0].size.as_deref(), Some("not a size"));

    let from_response = call(None, &response);
    assert_eq!(run(&from_response).1[0].size.as_deref(), Some("auto"));
}

#[test]
fn an_empty_size_or_quality_falls_through_to_the_next_source() {
    let options = params(&[("quality", text("hd"))]);
    let response = billing(1, Some(""), Some(""));
    let mut empty = call(None, &response);
    empty.size = Some("");
    empty.quality = Some("");
    empty.optional_params = Some(&options);
    let seen = &run(&empty).1[0];
    assert_eq!(seen.size.as_deref(), Some("1024-x-1024"));
    assert_eq!(seen.quality.as_deref(), Some("hd"));
}

#[test]
fn quality_prefers_the_explicit_value_then_the_response_then_a_string_option_then_standard() {
    let options = params(&[("quality", text("low"))]);
    let response = billing(1, None, Some("high"));
    let mut explicit = call(None, &response);
    explicit.quality = Some("hd");
    explicit.optional_params = Some(&options);
    assert_eq!(run(&explicit).1[0].quality.as_deref(), Some("hd"));

    let mut from_response = call(None, &response);
    from_response.optional_params = Some(&options);
    assert_eq!(run(&from_response).1[0].quality.as_deref(), Some("high"));

    let bare = billing(1, None, None);
    let mut from_options = call(None, &bare);
    from_options.optional_params = Some(&options);
    assert_eq!(run(&from_options).1[0].quality.as_deref(), Some("low"));

    let numeric = params(&[("quality", Json::Number(Number::from(3_i64)))]);
    let mut defaulted = call(None, &bare);
    defaulted.optional_params = Some(&numeric);
    assert_eq!(run(&defaulted).1[0].quality.as_deref(), Some("standard"));
}

#[rstest]
#[case::explicit_zero(Some(0), 3, 0)]
#[case::explicit_negative(Some(-2), 3, -2)]
#[case::explicit_positive(Some(5), 3, 5)]
#[case::returned_count(None, 3, 3)]
#[case::empty_data(None, 0, 0)]
fn an_explicit_count_beats_the_returned_data_length(
    #[case] count: Option<i64>,
    #[case] returned: i64,
    #[case] expected: i64,
) {
    let response = billing(returned, None, None);
    let mut explicit = call(None, &response);
    explicit.image_count = count;
    assert_eq!(run(&explicit).1[0].image_count, Some(expected));
}

#[rstest]
#[case::vertex(Some("vertex_ai"), "vertex_ai")]
#[case::recraft(Some("recraft"), "recraft")]
#[case::aiml(Some("aiml"), "aiml")]
#[case::cometapi(Some("cometapi"), "cometapi")]
#[case::gemini(Some("gemini"), "gemini")]
#[case::runwayml(Some("runwayml"), "runwayml")]
#[case::fal(Some("fal_ai"), "fal_ai")]
#[case::azure_ai(Some("azure_ai"), "azure_ai")]
#[case::bedrock(Some("bedrock"), "bedrock")]
#[case::openai_dalle(Some("openai"), "default")]
#[case::unmatched(Some("stability"), "default")]
#[case::no_provider(None, "default")]
fn each_provider_selects_its_own_calculator(#[case] provider: Option<&str>, #[case] label: &str) {
    let response = billing(1, None, None);
    let (cost, seen) = run(&call(provider, &response));
    assert_eq!(cost.unwrap(), Decimal::new(7, 1));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].label, label);
}

#[rstest]
#[case::generation(Some("image_generation"), "gemini")]
#[case::edit(Some("image_edit"), "gemini_edit")]
#[case::async_edit(Some("aimage_edit"), "gemini_edit")]
#[case::async_generation(Some("aimage_generation"), "gemini")]
#[case::passthrough(Some("passthrough-image-generation"), "gemini")]
#[case::unset(None, "gemini")]
fn gemini_edits_use_the_edit_calculator_only_for_the_two_edit_names(
    #[case] call_type: Option<&str>,
    #[case] label: &str,
) {
    let response = billing(1, None, None);
    let mut gemini = call(Some("gemini"), &response);
    gemini.call_type = call_type;
    assert_eq!(run(&gemini).1[0].label, label);
}

#[rstest]
#[case::openai_lower("openai", "gpt-image-1", "gpt_image")]
#[case::openai_mixed("openai", "GPT-Image-1.5", "gpt_image")]
#[case::azure("azure", "my-gpt-image-deployment", "gpt_image")]
#[case::dalle("openai", "dall-e-3", "default")]
#[case::azure_dalle("azure", "dall-e-3", "default")]
#[case::other_provider("anthropic", "gpt-image-1", "default")]
fn only_openai_and_azure_gpt_image_models_use_token_pricing(
    #[case] provider: &str,
    #[case] model: &str,
    #[case] label: &str,
) {
    let response = billing(1, None, None);
    let mut gpt = call(Some(provider), &response);
    gpt.model = model;
    let seen = &run(&gpt).1[0];
    assert_eq!(seen.label, label);
    if label == "gpt_image" {
        assert_eq!(seen.provider.as_deref(), Some(provider));
    }
}

#[test]
fn azure_ai_and_default_receive_the_resolved_arguments_and_bedrock_only_the_size() {
    let options = params(&[("size", text("8x8"))]);
    let response = billing(2, None, None);
    let mut azure = call(Some("azure_ai"), &response);
    azure.optional_params = Some(&options);
    let azure_seen = &run(&azure).1[0];
    assert_eq!(azure_seen.size.as_deref(), Some("8x8"));
    assert_eq!(azure_seen.image_count, Some(2));
    assert!(azure_seen.has_params);

    let mut bedrock = call(Some("bedrock"), &response);
    bedrock.optional_params = Some(&options);
    let bedrock_seen = &run(&bedrock).1[0];
    assert_eq!(bedrock_seen.size.as_deref(), Some("8x8"));
    assert!(bedrock_seen.has_params);

    let mut fal = call(Some("fal_ai"), &response);
    fal.optional_params = Some(&options);
    assert!(run(&fal).1[0].has_params);
    assert!(!run(&call(Some("fal_ai"), &response)).1[0].has_params);
}

#[test]
fn a_non_image_response_prices_vertex_at_zero_without_reaching_the_policy() {
    let binding = ImageBilling::default();
    let mut vertex = call(Some("vertex_ai"), &binding);
    vertex.response = ImageResponseView::Other;
    let (cost, seen) = run(&vertex);
    assert_eq!(cost.unwrap(), Decimal::ZERO);
    assert!(seen.is_empty());
}

#[test]
fn a_non_image_response_fails_bedrock_before_the_policy() {
    let binding = ImageBilling::default();
    let mut bedrock = call(Some("bedrock"), &binding);
    bedrock.response = ImageResponseView::Other;
    let (cost, seen) = run(&bedrock);
    assert!(matches!(cost, Err(ImageDispatchError::NotImageResponse)));
    assert!(seen.is_empty());
}

#[test]
fn a_non_image_response_still_reaches_the_other_calculators() {
    let binding = ImageBilling::default();
    let mut recraft = call(Some("recraft"), &binding);
    recraft.response = ImageResponseView::Other;
    let (cost, seen) = run(&recraft);
    assert_eq!(cost.unwrap(), Decimal::new(7, 1));
    assert_eq!(seen[0].label, "recraft");
}

const VALID_AND_INVALID_PRICES: &str =
    r#"{"output_cost_per_image":"0.04","input_cost_per_token":"abc","litellm_provider":"x"}"#;

#[rstest]
#[case::recraft("recraft")]
#[case::azure_ai("azure_ai")]
#[case::vertex("vertex_ai")]
#[case::openai("openai")]
fn only_valid_deployment_prices_reach_the_policy(#[case] provider: &str) {
    let info: ModelInfo = serde_json::from_str(VALID_AND_INVALID_PRICES).unwrap();
    let response = billing(1, None, None);
    let mut priced = call(Some(provider), &response);
    priced.model_info = Some(&info);
    assert_eq!(
        run(&priced).1[0].price_keys,
        Some(vec!["output_cost_per_image".to_owned()])
    );
}

#[test]
fn bedrock_never_receives_deployment_prices() {
    let info: ModelInfo = serde_json::from_str(VALID_AND_INVALID_PRICES).unwrap();
    let response = billing(1, None, None);
    let mut bedrock = call(Some("bedrock"), &response);
    bedrock.model_info = Some(&info);
    assert_eq!(run(&bedrock).1[0].price_keys, None);
}

#[test]
fn a_deployment_without_prices_passes_no_price_record() {
    let unrelated: ModelInfo = serde_json::from_str(r#"{"litellm_provider":"x"}"#).unwrap();
    let response = billing(1, None, None);
    let mut bare = call(Some("recraft"), &response);
    bare.model_info = Some(&unrelated);
    assert_eq!(run(&bare).1[0].price_keys, None);
}

#[test]
fn deployment_prices_are_exact_numbers() {
    let info: ModelInfo = serde_json::from_str(r#"{"output_cost_per_image":"0.0400"}"#).unwrap();
    let response = billing(1, None, None);
    let mut priced = call(Some("recraft"), &response);
    priced.model_info = Some(&info);
    let Some(Scalar::Number(number)) = run(&priced).1[0].image_price.clone() else {
        panic!("the price is not a number");
    };
    assert_eq!(number.to_string(), "0.0400");
}

#[test]
fn a_policy_failure_surfaces_unchanged() {
    let policy = Recording {
        seen: Mutex::new(Vec::new()),
        refuse: true,
    };
    let response = billing(1, None, None);
    let pattern = SizePattern::new().unwrap();
    let cost = route_image_cost(&policy, &pattern, &call(Some("recraft"), &response));
    assert!(matches!(cost, Err(ImageDispatchError::Policy(Refused))));
}

struct Search {
    seen: Mutex<Vec<(String, ToolUsage, ToolModelInfo)>>,
    answer: Option<Decimal>,
}

impl ToolPricePolicy for Search {
    fn web_search_cost(
        &self,
        provider: &str,
        usage: &ToolUsage,
        model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((provider.to_owned(), usage.clone(), model_info.clone()));
        self.answer
    }

    fn google_maps_cost(&self, _: &str, _: i64, _: &ToolModelInfo) -> Option<Decimal> {
        None
    }
}

#[rstest]
#[case::absent_usage(None, Some(Decimal::ONE), 0)]
#[case::zero_requests(Some(0), Some(Decimal::ONE), 0)]
fn image_grounding_without_requests_is_free_and_never_asks_the_dispatch(
    #[case] requests: Option<i64>,
    #[case] answer: Option<Decimal>,
    #[case] calls: usize,
) {
    let policy = Search {
        seen: Mutex::new(Vec::new()),
        answer,
    };
    let cost = image_grounding_cost(&policy, "gemini", requests, &ToolModelInfo::default());
    assert_eq!(cost, Decimal::ZERO);
    assert_eq!(policy.seen.lock().unwrap().len(), calls);
}

#[rstest]
#[case::priced(Some(Decimal::new(105, 2)), Decimal::new(105, 2))]
#[case::unsupported(None, Decimal::ZERO)]
#[case::already_billed(Some(Decimal::ZERO), Decimal::ZERO)]
fn image_grounding_adapts_the_count_to_a_prompt_detail_and_defaults_a_falsey_answer(
    #[case] answer: Option<Decimal>,
    #[case] expected: Decimal,
) {
    let policy = Search {
        seen: Mutex::new(Vec::new()),
        answer,
    };
    let info = ToolModelInfo {
        litellm_provider: Some("gemini".to_owned()),
        search_context: SearchContextRates {
            low: Rate::Absent,
            medium: Rate::Absent,
            high: Rate::Absent,
        },
        file_search_per_gb_per_day: Rate::Absent,
        file_search_per_1k_calls: Rate::Absent,
        vector_store_per_gb_per_day: Rate::Absent,
        computer_input_per_1k: Rate::Absent,
        computer_output_per_1k: Rate::Absent,
        code_interpreter_per_session: Rate::Absent,
    };
    assert_eq!(
        image_grounding_cost(&policy, "vertex_ai", Some(3), &info),
        expected
    );
    let seen = mem::take(&mut *policy.seen.lock().unwrap());
    let (provider, usage, passed) = &seen[0];
    assert_eq!(provider, "vertex_ai");
    assert_eq!(passed, &info);
    assert_eq!(
        usage
            .cost
            .prompt_details
            .and_then(|details| details.web_search_requests),
        Some(3)
    );
    assert_eq!(usage.cost.prompt_tokens, 0);
    assert_eq!(usage.server_tool_use, None);
}

#[test]
fn a_negative_request_count_is_truthy_and_reaches_the_dispatch() {
    let policy = Search {
        seen: Mutex::new(Vec::new()),
        answer: Some(Decimal::TWO),
    };
    let cost = image_grounding_cost(&policy, "gemini", Some(-1), &ToolModelInfo::default());
    assert_eq!(cost, Decimal::TWO);
}
