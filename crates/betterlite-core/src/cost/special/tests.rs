#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::cost_adjustments::{AdjustmentEntry, MarginFieldEntry};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::billing_text::{ChoiceText, ResponseText};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::protocol_cost::{A2aCall, A2aUsage, McpCall, ProtocolCall};
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use betterlite_models::resolution::{ProviderIdentity, ResolvedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::usage_sum::Usage;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::error::SpecialChargeError;
use super::policy::{ModelResolver, SearchQuery, SpecialPricePolicy};
use super::replicate::ReplicateStamps;
use super::request::{
    HiddenDuration, ImageParts, LoggingView, RealtimeSession, SpecialOutcome, SpecialRequest,
    SpecialResponse, SpecialServices, TokenStage, TranscriptionDuration, VideoUsage,
};
use super::special_charge;
use crate::cost::call_price::policy::error::PolicyError;
use crate::cost::image_dispatch::options::SizePattern;
use crate::cost::image_dispatch::policy::{ImagePricePolicy, ImageResponseView, ImageRoute};
use crate::cost::prepare::error::PrepareError;
use crate::cost::prepare::{AttemptBilling, AttemptOutcome, PreparedBillingAttempt, RerankUnits};
use crate::routes::CallType;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture is a decimal")
}

fn record(fields: &[(&str, &str)]) -> PriceRecord {
    fields
        .iter()
        .map(|(key, value)| {
            let number = Number::from_str(value).expect("the fixture is a number");
            ((*key).to_owned(), Scalar::Number(number))
        })
        .collect()
}

struct Catalog(BTreeMap<String, PriceRecord>);

impl PriceLookup for Catalog {
    fn effective_model_info(&self, model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.0.get(key).cloned()
    }

    fn effective_priced_model(&self, model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        self.0.get(model).map(|entry| PricedModel {
            record: entry.clone(),
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

struct Resolver(Option<ResolvedModel>);

impl ModelResolver for Resolver {
    type Failure = &'static str;

    fn resolve_model(&self, _model: &str) -> Result<ResolvedModel, &'static str> {
        self.0.clone().ok_or("the model names no provider")
    }
}

struct Images;

impl ImagePricePolicy for Images {
    type Error = Infallible;

    fn image_cost(&self, _route: &ImageRoute<'_>) -> Result<Decimal, Infallible> {
        Ok(dec("0.7"))
    }
}

// Records every policy call so a test sees what was asked, and in which order.
struct Policy {
    registry_has_price: bool,
    log: Mutex<Vec<String>>,
}

impl Policy {
    fn note(&self, entry: String) {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

impl SpecialPricePolicy for Policy {
    fn search_cost(&self, query: &SearchQuery<'_>) -> Result<GenericCostResult, PolicyError> {
        let has_params = query.optional_params.is_some();
        self.note(format!(
            "search:{}:{}:{has_params}",
            query.model, query.number_of_queries
        ));
        Ok(GenericCostResult {
            prompt_cost: dec("0.002"),
            completion_cost: dec("0.003"),
        })
    }

    fn has_together_registry_pricing(
        &self,
        model: &str,
        _lookup: &dyn PriceLookup,
    ) -> Result<bool, PolicyError> {
        self.note(format!("registry:{model}"));
        Ok(self.registry_has_price)
    }

    fn together_category(&self, model: &str, call_type: CallType) -> Result<String, PolicyError> {
        self.note(format!("category:{model}:{call_type:?}"));
        Ok("together-category".to_owned())
    }
}

#[derive(Debug, Default, PartialEq)]
struct Summary {
    amounts: [Option<Decimal>; 7],
    vertex_location: Option<String>,
    data_residency: Option<String>,
    service_tier: Option<String>,
}

#[derive(Default)]
struct Stored(Option<Summary>);

impl CostBreakdownTarget for Stored {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.0 = Some(Summary {
            amounts: [
                Some(input.input_cost),
                Some(input.output_cost),
                Some(input.total_cost),
                Some(input.tool_usage_cost),
                input.original_cost,
                input.discount_amount,
                input.margin_total_amount,
            ],
            vertex_location: input.vertex_location.clone(),
            data_residency: input.data_residency.clone(),
            service_tier: input.service_tier.clone(),
        });
        Ok(())
    }
}

struct Fixture {
    lookup: Catalog,
    resolver: Resolver,
    policy: Policy,
    pattern: SizePattern,
    database: TimeZoneDatabase,
    models: ProviderModelSets,
    discount: BTreeMap<String, AdjustmentEntry>,
    margin: BTreeMap<String, AdjustmentEntry>,
}

type Failure = SpecialChargeError<Infallible>;

impl Fixture {
    fn new(prices: &[(&str, &[(&str, &str)])]) -> Self {
        Self {
            lookup: Catalog(
                prices
                    .iter()
                    .map(|(key, fields)| ((*key).to_owned(), record(fields)))
                    .collect(),
            ),
            resolver: Resolver(None),
            policy: Policy {
                registry_has_price: false,
                log: Mutex::new(Vec::new()),
            },
            pattern: SizePattern::new().expect("the size pattern compiles"),
            database: TimeZoneDatabase::from_env(),
            models: ProviderModelSets::default(),
            discount: BTreeMap::new(),
            margin: BTreeMap::new(),
        }
    }

    fn log(&self) -> Vec<String> {
        self.policy
            .log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn run_with<Clock>(
        &self,
        request: &SpecialRequest<'_>,
        attempt: PreparedBillingAttempt,
        clock: &mut Clock,
        target: Option<&mut Stored>,
    ) -> Result<SpecialOutcome, Failure>
    where
        Clock: FnMut() -> SystemTime,
    {
        let services = SpecialServices {
            lookup: &self.lookup,
            resolver: &self.resolver,
            image_policy: &Images,
            special_policy: &self.policy,
            size_pattern: &self.pattern,
            database: &self.database,
            replicate_models: &self.models,
            replicate_price: dec("0.0014"),
            discount: &self.discount,
            margin: &self.margin,
        };
        special_charge(&services, request, attempt, clock, target)
    }

    fn run(
        &self,
        request: &SpecialRequest<'_>,
        attempt: PreparedBillingAttempt,
    ) -> Result<SpecialOutcome, Failure> {
        self.run_with(request, attempt, &mut || UNIX_EPOCH, None)
    }

    fn charge(&self, request: &SpecialRequest<'_>, model: &str, provider: Option<&str>) -> Decimal {
        let outcome = self.run(request, attempt(Some(model), provider));
        let Ok(SpecialOutcome::Charged(charge)) = outcome else {
            panic!("no early charge");
        };
        charge
    }

    fn stage(
        &self,
        request: &SpecialRequest<'_>,
        model: &str,
        provider: Option<&str>,
    ) -> TokenStage {
        match self.run(request, attempt(Some(model), provider)) {
            Ok(SpecialOutcome::Continue(stage)) => *stage,
            Ok(SpecialOutcome::Charged(_) | SpecialOutcome::Failed(_)) | Err(_) => {
                panic!("the call did not continue")
            }
        }
    }
}

fn attempt(model: Option<&str>, provider: Option<&str>) -> PreparedBillingAttempt {
    PreparedBillingAttempt {
        model: model.map(str::to_owned),
        custom_llm_provider: provider.map(str::to_owned),
        region_name: None,
        outcome: AttemptOutcome::Ready(AttemptBilling {
            prompt_tokens: 0,
            completion_tokens: 9,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            prompt_characters: None,
            completion_characters: None,
            transcription_duration: Decimal::ZERO,
            rerank_units: None,
        }),
    }
}

fn billing(stage: &TokenStage) -> &AttemptBilling {
    match &stage.attempt.outcome {
        AttemptOutcome::Ready(found) => found,
        AttemptOutcome::Failed(_) => panic!("the attempt failed"),
    }
}

fn request(call_type: CallType) -> SpecialRequest<'static> {
    SpecialRequest {
        call_type,
        passthrough_image: false,
        prompt: "",
        messages: &[],
        optional_params: None,
        custom_pricing: None,
        router_model_id: None,
        base_model: None,
        selected_model: None,
        service_tier: None,
        data_residency: None,
        total_time: None,
        moment: DateTime::<Utc>::from_timestamp(0, 0).expect("the epoch"),
        logging: LoggingView {
            model: None,
            params: None,
            timing: None,
        },
        response: SpecialResponse {
            cost_response: None,
            image: ImageParts {
                response: ImageResponseView::Other,
                quality: None,
                image_count: None,
                size: None,
            },
            video_usage: None,
            transcription: TranscriptionDuration::default(),
            rerank: None,
            realtime: None,
            protocol: None,
            replicate: ReplicateStamps::default(),
            model_response: None,
        },
    }
}

fn mcp(response_cost: Json, default_cost: Json) -> ProtocolCall {
    ProtocolCall::Mcp(McpCall {
        response_cost,
        tool_name: "lookup".to_owned(),
        default_cost_per_query: default_cost,
        tool_name_to_cost_per_query: IndexMap::new(),
    })
}

fn number(text: &str) -> Json {
    Json::Number(Number::from_str(text).expect("the fixture is a number"))
}

#[rstest]
#[case::send(CallType::SendMessage)]
#[case::async_send(CallType::AsendMessage)]
fn an_agent_call_ends_before_the_model_check(#[case] call_type: CallType) {
    let call = ProtocolCall::A2a(A2aCall {
        response_cost: number("0.25"),
        cost_per_query: Json::Null,
        input_cost_per_token: Json::Null,
        output_cost_per_token: Json::Null,
        usage: A2aUsage::Absent,
    });
    let mut priced = request(call_type);
    priced.response.protocol = Some(&call);
    let fixture = Fixture::new(&[]);
    let outcome = fixture.run(&priced, attempt(None, None)).unwrap();
    assert!(matches!(outcome, SpecialOutcome::Charged(charge) if charge == dec("0.25")));
    let unrecorded = fixture
        .run(&request(call_type), attempt(None, None))
        .unwrap();
    assert!(matches!(unrecorded, SpecialOutcome::Charged(charge) if charge.is_zero()));
}

#[test]
fn a_call_without_a_model_is_refused_and_a_failed_attempt_passes_through() {
    let fixture = Fixture::new(&[]);
    let call = request(CallType::Completion);
    let missing = fixture.run(&call, attempt(None, None));
    assert!(matches!(missing, Err(SpecialChargeError::ModelMissing)));
    let mut failed = attempt(None, None);
    failed.outcome = AttemptOutcome::Failed(PrepareError::ModelMissing);
    let passed = fixture.run(&call, failed).unwrap();
    assert!(matches!(passed, SpecialOutcome::Failed(_)));
}

#[rstest]
#[case::inferred(None, "openai/gpt-4o", Some("openai"), "gpt-4o", Some("openai"))]
#[case::given(
    Some("azure"),
    "openai/gpt-4o",
    Some("openai"),
    "openai/gpt-4o",
    Some("azure")
)]
#[case::unresolved(None, "gpt-4o", None, "gpt-4o", None)]
fn the_provider_is_inferred_only_when_none_was_given(
    #[case] provider: Option<&str>,
    #[case] model: &str,
    #[case] resolved_provider: Option<&str>,
    #[case] expected_model: &str,
    #[case] expected_provider: Option<&str>,
) {
    let mut fixture = Fixture::new(&[]);
    fixture.resolver = Resolver(resolved_provider.map(|_| ResolvedModel {
        model: "gpt-4o".to_owned(),
        provider: ProviderIdentity::Known(ProviderId::Openai),
        api_base: None,
        api_key: None,
    }));
    let stage = fixture.stage(&request(CallType::Completion), model, provider);
    assert_eq!(stage.attempt.model.as_deref(), Some(expected_model));
    assert_eq!(
        stage.attempt.custom_llm_provider.as_deref(),
        expected_provider
    );
}

#[rstest]
#[case::generation(CallType::ImageGeneration, false, true)]
#[case::async_generation(CallType::AimageGeneration, false, true)]
#[case::edit(CallType::ImageEdit, false, true)]
#[case::async_edit(CallType::AimageEdit, false, true)]
#[case::other_call(CallType::Completion, false, false)]
#[case::passthrough(CallType::Completion, true, true)]
fn only_an_image_call_with_a_typed_response_is_priced_here(
    #[case] call_type: CallType,
    #[case] passthrough: bool,
    #[case] priced: bool,
) {
    let image = ImageBilling::default();
    let mut call = request(call_type);
    call.passthrough_image = passthrough;
    call.response.image.response = ImageResponseView::Image(&image);
    let fixture = Fixture::new(&[]);
    let outcome = fixture.run(&call, attempt(Some("dall-e-3"), None)).unwrap();
    assert_eq!(matches!(outcome, SpecialOutcome::Charged(_)), priced);
    call.response.image.response = ImageResponseView::Other;
    let untyped = fixture.run(&call, attempt(Some("dall-e-3"), None)).unwrap();
    assert!(matches!(untyped, SpecialOutcome::Continue(_)));
}

fn video_usage(duration: Option<&str>, count: Option<i64>) -> VideoUsage<'static> {
    VideoUsage {
        duration_seconds: duration.map(dec),
        video_resolution: None,
        provider_reported_cost_usd: None,
        video_count: count,
    }
}

#[rstest]
#[case::single((Some("4"), None), None, None, "0.4")]
#[case::counted((Some("4"), Some(3)), None, None, "1.2")]
#[case::one_video((Some("4"), Some(1)), None, None, "0.4")]
#[case::zero_videos((Some("4"), Some(0)), None, None, "0.4")]
#[case::negative_videos((Some("4"), Some(-2)), None, None, "0.4")]
#[case::resolution_rate((Some("4"), None), Some(" 1080P "), None, "0.8")]
#[case::provider_cost((Some("4"), None), None, Some("9"), "9")]
#[case::free_provider_cost((Some("4"), None), None, Some("0"), "0")]
#[case::no_duration((None, None), None, None, "0")]
fn a_video_call_is_priced_by_duration_or_the_provider_cost(
    #[values(
        CallType::CreateVideo,
        CallType::AcreateVideo,
        CallType::VideoEdit,
        CallType::AvideoEdit,
        CallType::VideoRemix,
        CallType::AvideoRemix
    )]
    call_type: CallType,
    #[case] duration_and_count: (Option<&str>, Option<i64>),
    #[case] resolution: Option<&'static str>,
    #[case] reported: Option<&str>,
    #[case] expected: &str,
) {
    let fixture = Fixture::new(&[(
        "sora-2",
        &[
            ("output_cost_per_second", "0.1"),
            ("output_cost_per_second_1080p", "0.2"),
        ],
    )]);
    let (duration, count) = duration_and_count;
    let mut call = request(call_type);
    call.response.video_usage = Some(VideoUsage {
        video_resolution: resolution,
        provider_reported_cost_usd: reported.map(dec),
        ..video_usage(duration, count)
    });
    assert_eq!(fixture.charge(&call, "sora-2", None), dec(expected));
    let dictionary = request(call_type);
    assert_eq!(fixture.charge(&dictionary, "sora-2", None), Decimal::ZERO);
}

#[test]
fn a_deployment_price_record_outranks_the_provider_cost_and_an_unknown_model_fails() {
    let fixture = Fixture::new(&[]);
    let info = Json::Object(IndexMap::from([(
        "model_info".to_owned(),
        Json::Object(IndexMap::from([(
            "output_cost_per_video_per_second".to_owned(),
            number("2"),
        )])),
    )]));
    let params = IndexMap::from([("metadata".to_owned(), info)]);
    let mut call = request(CallType::CreateVideo);
    call.custom_pricing = Some(true);
    call.logging.params = Some(&params);
    call.response.video_usage = Some(VideoUsage {
        provider_reported_cost_usd: Some(dec("9")),
        ..video_usage(Some("3"), None)
    });
    assert_eq!(fixture.charge(&call, "sora-2", None), dec("6"));
    let mut unpriced = request(CallType::CreateVideo);
    unpriced.response.video_usage = Some(video_usage(Some("3"), None));
    let outcome = fixture.run(&unpriced, attempt(Some("mystery"), None));
    assert!(matches!(outcome, Err(SpecialChargeError::Video(_))));
}

#[rstest]
#[case::speech(CallType::Speech, Some(3))]
#[case::async_speech(CallType::Aspeech, Some(3))]
#[case::completion(CallType::Completion, None)]
fn speech_records_the_prompt_characters(
    #[case] call_type: CallType,
    #[case] expected: Option<u64>,
) {
    let mut call = request(call_type);
    call.prompt = "a b\tc";
    let stage = Fixture::new(&[]).stage(&call, "tts-1", None);
    assert_eq!(billing(&stage).prompt_characters, expected);
}

#[rstest]
#[case::missing_key(HiddenDuration::Missing, Some("5"), "5")]
#[case::missing_everywhere(HiddenDuration::Missing, None, "0")]
#[case::hidden_wins(HiddenDuration::Seconds(dec("7")), Some("5"), "7")]
#[case::null_blocks_the_fallback(HiddenDuration::Null, Some("5"), "0")]
fn transcription_reads_the_hidden_duration_before_the_response(
    #[values(CallType::Transcription, CallType::Atranscription)] call_type: CallType,
    #[case] hidden: HiddenDuration,
    #[case] response_duration: Option<&str>,
    #[case] expected: &str,
) {
    let mut call = request(call_type);
    call.response.transcription = TranscriptionDuration {
        hidden,
        response_duration: response_duration.map(dec),
    };
    let stage = Fixture::new(&[]).stage(&call, "whisper-1", None);
    assert_eq!(billing(&stage).transcription_duration, dec(expected));
}

#[rstest]
#[case::units(Some(3), 3)]
#[case::zero_falls_back(Some(0), 1)]
#[case::missing_falls_back(None, 1)]
fn rerank_stores_both_units_and_defaults_the_query_count(
    #[values(CallType::Rerank, CallType::Arerank)] call_type: CallType,
    #[case] search_units: Option<i64>,
    #[case] completion_tokens: i64,
) {
    let fixture = Fixture::new(&[]);
    let mut call = request(call_type);
    call.response.rerank = Some(RerankUnits {
        search_units,
        total_tokens: Some(40),
    });
    let stage = fixture.stage(&call, "rerank-v3", None);
    assert_eq!(billing(&stage).completion_tokens, completion_tokens);
    let units = billing(&stage).rerank_units.as_ref().expect("units");
    assert_eq!(
        (units.search_units, units.total_tokens),
        (search_units, Some(40))
    );
    let untyped = fixture.stage(&request(CallType::Rerank), "rerank-v3", None);
    assert_eq!(billing(&untyped).completion_tokens, 9);
    assert!(billing(&untyped).rerank_units.is_none());
}

#[rstest]
#[case::two_queries(Some(Json::List(vec![number("1"), number("2")])), 2)]
#[case::empty_list(Some(Json::List(Vec::new())), 0)]
#[case::text(Some(Json::Text("rust".to_owned())), 1)]
#[case::no_query(None, 1)]
fn search_counts_a_list_query_by_its_length(#[case] query: Option<Json>, #[case] expected: i64) {
    let params: IndexMap<String, Json> = query
        .into_iter()
        .map(|found| ("query".to_owned(), found))
        .collect();
    let fixture = Fixture::new(&[]);
    let mut call = request(CallType::Search);
    call.optional_params = Some(&params);
    fixture.charge(&call, "search-api", Some("tavily"));
    assert_eq!(
        fixture.log(),
        [format!("search:tavily/search:{expected}:true")]
    );
}

#[rstest]
#[case::slash_free("search-api", Some("tavily"), "tavily/search")]
#[case::already_prefixed("tavily/search", Some("tavily"), "tavily/search")]
#[case::other_prefix("exa/api", Some("tavily"), "exa/api")]
#[case::empty_provider("search-api", Some(""), "search-api")]
#[case::no_provider("search-api", None, "search-api")]
fn search_builds_the_catalog_name_from_a_provider_and_a_bare_model(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: &str,
) {
    let fixture = Fixture::new(&[]);
    fixture.charge(&request(CallType::Asearch), model, provider);
    assert_eq!(fixture.log(), [format!("search:{expected}:1:false")]);
}

#[test]
fn search_applies_the_discount_then_the_margin_and_stores_the_breakdown() {
    let mut fixture = Fixture::new(&[]);
    fixture
        .discount
        .insert("tavily".to_owned(), AdjustmentEntry::Percent(dec("0.1")));
    fixture.margin.insert(
        "global".to_owned(),
        AdjustmentEntry::Split {
            percentage: MarginFieldEntry::Absent,
            fixed_amount: MarginFieldEntry::Number(dec("0.01")),
        },
    );
    let mut call = request(CallType::Search);
    call.service_tier = Some("flex");
    call.data_residency = Some("eu");
    let mut stored = Stored::default();
    let outcome = fixture
        .run_with(
            &call,
            attempt(Some("tavily/search"), Some("tavily")),
            &mut || UNIX_EPOCH,
            Some(&mut stored),
        )
        .unwrap();
    assert!(matches!(outcome, SpecialOutcome::Charged(charge) if charge == dec("0.0145")));
    let amounts = ["0.002", "0.003", "0.0145", "0", "0.005", "0.0005", "0.01"];
    let expected = Summary {
        amounts: amounts.map(|text| Some(dec(text))),
        vertex_location: None,
        data_residency: Some("eu".to_owned()),
        service_tier: Some("flex".to_owned()),
    };
    assert_eq!(stored.0, Some(expected));
}

#[rstest]
#[case::custom_pricing(Some(true), None, "0.2")]
#[case::base_model(None, Some("deployment"), "0.2")]
#[case::both(Some(true), Some("deployment"), "0.2")]
#[case::neither(None, None, "0.02")]
fn realtime_prices_the_selected_model_only_for_custom_or_base_pricing(
    #[case] custom_pricing: Option<bool>,
    #[case] base_model: Option<&'static str>,
    #[case] expected: &str,
) {
    let fixture = Fixture::new(&[
        (
            "rt-model",
            &[
                ("input_cost_per_token", "0.001"),
                ("output_cost_per_token", "0.002"),
            ],
        ),
        (
            "selected",
            &[
                ("input_cost_per_token", "0.01"),
                ("output_cost_per_token", "0.02"),
            ],
        ),
    ]);
    let usage = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    };
    let mut call = request(CallType::PrivateArealtime);
    call.custom_pricing = custom_pricing;
    call.base_model = base_model;
    call.selected_model = Some("selected");
    call.response.realtime = Some(RealtimeSession {
        results: &[],
        usage: Some(&usage),
    });
    assert_eq!(
        fixture.charge(&call, "rt-model", Some("openai")),
        dec(expected)
    );
}

#[rstest]
#[case::no_usage(false, Some("openai"))]
#[case::no_provider(true, None)]
fn realtime_needs_a_usage_and_a_provider(#[case] has_usage: bool, #[case] provider: Option<&str>) {
    let usage = Usage::default();
    let mut call = request(CallType::PrivateArealtime);
    call.response.realtime = Some(RealtimeSession {
        results: &[],
        usage: has_usage.then_some(&usage),
    });
    let outcome = Fixture::new(&[]).run(&call, attempt(Some("rt-model"), provider));
    assert!(matches!(outcome, Err(SpecialChargeError::RealtimeInputs)));
    let without_session = request(CallType::PrivateArealtime);
    let ordinary = Fixture::new(&[]).run(&without_session, attempt(Some("rt-model"), provider));
    assert!(matches!(ordinary, Ok(SpecialOutcome::Continue(_))));
}

#[test]
fn a_tool_call_charges_its_price_and_a_missing_record_is_free() {
    let fixture = Fixture::new(&[]);
    let priced = mcp(Json::Null, number("0.5"));
    let free = mcp(Json::Null, number("0"));
    let mut call = request(CallType::CallMcpTool);
    call.response.protocol = Some(&priced);
    assert_eq!(fixture.charge(&call, "lookup", None), dec("0.5"));
    call.response.protocol = Some(&free);
    assert_eq!(fixture.charge(&call, "lookup", None), Decimal::ZERO);
    call.response.protocol = None;
    assert_eq!(fixture.charge(&call, "lookup", None), Decimal::ZERO);
}

#[rstest]
#[case::model_name("togethercomputer/llama", None, false, true)]
#[case::provider_name("llama", Some("together_ai"), false, true)]
#[case::registry_price("together_ai/llama", None, true, false)]
#[case::unrelated("gpt-4o", Some("openai"), false, false)]
fn the_together_category_replaces_a_model_the_registry_cannot_price(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] registry_has_price: bool,
    #[case] replaced: bool,
) {
    let mut fixture = Fixture::new(&[]);
    fixture.policy.registry_has_price = registry_has_price;
    let stage = fixture.stage(&request(CallType::Completion), model, provider);
    let expected = if replaced { "together-category" } else { model };
    assert_eq!(stage.attempt.model.as_deref(), Some(expected));
    let log = fixture.log();
    assert_eq!(log.is_empty(), model == "gpt-4o");
    assert_eq!(
        log.iter().any(|entry| entry.starts_with("category:")),
        replaced
    );
    if replaced {
        assert_eq!(log[1], format!("category:{model}:Completion"));
    }
}

// The clock answers 10s, 20s, 30s in turn, so the first read is the created default and the second the ended one.
#[rstest]
#[case::reported_time("replicate/llama", Some("2000"), (None, None), "0.0028", 3)]
#[case::both_stamps("replicate/llama", None, (Some("100"), Some("105")), "0.000007", 1)]
#[case::default_created_first("replicate/llama", None, (None, Some("5")), "-0.000007", 1)]
#[case::default_ended("replicate/llama", None, (Some("100"), None), "-0.000112", 1)]
#[case::beside_together("togethercomputer/replicate-x", Some("1000"), (None, None), "0.0014", 3)]
fn an_unmapped_replicate_model_is_billed_by_its_time(
    #[case] model: &str,
    #[case] total_time: Option<&str>,
    #[case] stamps: (Option<&str>, Option<&str>),
    #[case] expected: &str,
    #[case] unread_ticks: usize,
) {
    let mut fixture = Fixture::new(&[]);
    fixture.policy.registry_has_price = true;
    let mut call = request(CallType::Completion);
    call.total_time = total_time.map(dec);
    call.response.replicate = ReplicateStamps {
        created: stamps.0.map(dec),
        ended: stamps.1.map(dec),
    };
    let mut ticks = [10_u64, 20, 30].into_iter();
    let mut clock = || {
        let seconds = ticks.next().expect("at most two reads");
        UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .expect("the time is in range")
    };
    let outcome = fixture
        .run_with(&call, attempt(Some(model), None), &mut clock, None)
        .unwrap();
    assert!(matches!(outcome, SpecialOutcome::Charged(charge) if charge == dec(expected)));
    assert_eq!(ticks.len(), unread_ticks);
}

#[test]
fn replicate_billing_skips_catalog_models_and_covers_the_replicate_model_set() {
    let mut fixture = Fixture::new(&[("replicate/llama", &[("input_cost_per_token", "0.1")])]);
    let mut call = request(CallType::Completion);
    call.total_time = Some(dec("2000"));
    assert!(matches!(
        fixture.run(&call, attempt(Some("replicate/llama"), None)),
        Ok(SpecialOutcome::Continue(_))
    ));
    assert!(matches!(
        fixture.run(&call, attempt(Some("plain-llama"), None)),
        Ok(SpecialOutcome::Continue(_))
    ));
    fixture
        .models
        .sets
        .entry(ProviderModelSet::Replicate)
        .or_default()
        .insert("plain-llama".to_owned());
    assert_eq!(fixture.charge(&call, "plain-llama", None), dec("0.0028"));
}

#[test]
fn a_vertex_call_counts_the_prompt_and_response_characters() {
    let message = Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text("hi there".to_owned())),
    ]));
    let reply = Json::Text("abc d".to_owned());
    let choices = [ChoiceText::Message {
        content: Some(&reply),
    }];
    let text = ResponseText {
        event: None,
        choices: &choices,
    };
    let messages = [message];
    let mut call = request(CallType::Completion);
    call.messages = &messages;
    call.response.model_response = Some(&text);
    let fixture = Fixture::new(&[]);
    let vertex = fixture.stage(&call, "gemini-pro", Some("vertex_ai"));
    assert_eq!(billing(&vertex).prompt_characters, Some(7));
    assert_eq!(billing(&vertex).completion_characters, Some(4));
    let beta = fixture.stage(&call, "gemini-pro", Some("vertex_ai_beta"));
    assert_eq!(billing(&beta).prompt_characters, None);
    assert_eq!(billing(&beta).completion_characters, None);
}

#[test]
fn the_token_stage_receives_the_response_time_and_the_logged_model() {
    let fixture = Fixture::new(&[]);
    let mut call = request(CallType::Completion);
    call.total_time = Some(dec("1500"));
    call.logging.model = Some("logged-model");
    let stage = fixture.stage(&call, "gpt-4o", Some("openai"));
    assert_eq!(stage.response_time_ms, dec("1500"));
    assert_eq!(
        stage.request_model_for_cost.as_deref(),
        Some("logged-model")
    );
    let unlogged = fixture.stage(&request(CallType::Completion), "gpt-4o", Some("openai"));
    assert_eq!(unlogged.request_model_for_cost, None);
}

struct Uninstalled;

impl SpecialPricePolicy for Uninstalled {}

#[test]
fn an_uninstalled_policy_answers_every_operation_with_its_unavailable_adapter() {
    let message =
        |adapter: &str| format!("betterlite has no executable adapter for the {adapter} provider");
    let query = SearchQuery {
        model: "tavily/search",
        provider: Some("tavily"),
        number_of_queries: 1,
        optional_params: None,
    };
    let search = Uninstalled.search_cost(&query).unwrap_err();
    assert_eq!(search.to_string(), message("tavily"));
    let registry = Uninstalled
        .has_together_registry_pricing("together_ai/llama", &Catalog(BTreeMap::new()))
        .unwrap_err();
    assert_eq!(registry.to_string(), message("together_ai"));
    let category = Uninstalled
        .together_category("together_ai/llama", CallType::Completion)
        .unwrap_err();
    assert_eq!(category.to_string(), message("together_ai"));
}
