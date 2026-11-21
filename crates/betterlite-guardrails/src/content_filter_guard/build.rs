use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use betterlite_config::sections::content_category::Settings as CategorySettings;
use betterlite_config::sections::content_category::loading::{CategoryLoadResult, load_categories};
use betterlite_config::sections::content_filter_catalog::roots::{DATA_DIR, data_roots};
use betterlite_config::sections::content_filter_records::{
    BlockedWord, ContentFilterAction, ContentFilterCategoryConfig, ContentFilterPattern,
};
use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use crate::content_filter::competitor_intent_airline::settings::AirlineSettings;
use crate::content_filter::competitor_intent_base::BaseCompetitorIntentChecker;
use crate::content_filter::competitor_intent_base::CompetitorIntentError;
use crate::content_filter::competitor_intent_base::configuration::Settings as CompetitorSettings;
use crate::content_filter_init::args::ContentFilterArgs;
use crate::content_filter_inputs::images::ImageDescriptionCompletion;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::pattern_matching::CompiledPattern;
use crate::pattern_matching::blocked_words::load as load_blocked_words;
use crate::pattern_matching::compile;

use super::error::ContentFilterBuildError;
use super::{
    CompetitorIntentChecker, ContentFilterGuardrail, GUARDRAIL_PROVIDER, KEYWORD_REDACTION_TAG,
    PATTERN_REDACTION_FORMAT, SUPPORTED_EVENT_HOOKS,
};

const DEFAULT_EVENT_HOOK: &str = "pre_call";
const DURING_CALL: &str = "during_call";
const GENERIC_COMPETITOR: &str = "generic";
// LiteLLM's ContentFilterGuardrail records its own apply_guardrail run in a finally block
const CONTENT_FILTER_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: true,
    uses_apply_guardrail_interface: true,
    deny_text_on_error: false,
};

#[derive(Debug, Default)]
struct CompetitorTemplates {
    refuse: Option<String>,
    reframe: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum CompetitorBuildError {
    #[error("invalid competitor intent config: {0}")]
    Config(#[from] serde_json::Error),
    #[error(transparent)]
    Checker(#[from] CompetitorIntentError),
}

// The keys LiteLLM's two competitor checkers read, plus the five only the airline one reads
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CompetitorConfig {
    competitor_intent_type: Option<String>,
    brand_self: Option<Vec<String>>,
    competitors: Option<Vec<String>>,
    competitor_aliases: Option<BTreeMap<String, Vec<String>>>,
    locations: Option<Vec<String>>,
    policy: Option<BTreeMap<String, String>>,
    threshold_high: Option<f64>,
    threshold_medium: Option<f64>,
    threshold_low: Option<f64>,
    reframe_message_template: Option<String>,
    refuse_message_template: Option<String>,
    comparison_words: Option<Vec<String>>,
    domain_words: Option<Vec<String>>,
    other_meaning_signals: Option<Vec<String>>,
    competitor_signals: Option<Vec<String>>,
    other_meaning_anchors: Option<Vec<String>>,
    explicit_competitor_marker: Option<String>,
    explicit_other_meaning_marker: Option<String>,
}

impl CompetitorConfig {
    fn settings(&self) -> CompetitorSettings {
        let defaults = CompetitorSettings::default();
        CompetitorSettings {
            brand_self: self.brand_self.clone().unwrap_or_default(),
            competitors: self.competitors.clone().unwrap_or_default(),
            competitor_aliases: self.competitor_aliases.clone().unwrap_or_default(),
            locations: self.locations.clone().unwrap_or_default(),
            policy: self.policy.clone().unwrap_or_default(),
            threshold_high: self.threshold_high.unwrap_or(defaults.threshold_high),
            threshold_medium: self.threshold_medium.unwrap_or(defaults.threshold_medium),
            threshold_low: self.threshold_low.unwrap_or(defaults.threshold_low),
            reframe_message_template: self.reframe_message_template.clone(),
            refuse_message_template: self.refuse_message_template.clone(),
            comparison_words: self.comparison_words.clone().unwrap_or_default(),
            domain_words: self.domain_words.clone().unwrap_or_default(),
        }
    }
}

impl<R: ImageDescriptionCompletion> ContentFilterGuardrail<R> {
    pub fn new(
        args: ContentFilterArgs,
        router: Option<R>,
    ) -> Result<Self, ContentFilterBuildError> {
        let base = GuardrailBase::new(guardrail_settings(&args), true)?;
        let (competitor_intent_checker, templates) =
            competitor_checker(args.competitor_intent_config.as_ref());
        let loaded_categories = guardrail_categories(args.categories.as_deref());
        let compiled_patterns = compile_patterns(args.patterns.as_deref().unwrap_or_default())?;
        warn_on_during_call_mask(&base, &compiled_patterns);
        let blocked_words = blocked_word_map(
            args.blocked_words.as_deref().unwrap_or_default(),
            args.blocked_words_file.as_deref(),
        )?;
        Ok(Self {
            base,
            guardrail_provider: GUARDRAIL_PROVIDER.to_owned(),
            config_guardrail_id: args.guardrail_id.clone(),
            config_policy_template: args.policy_template.clone(),
            pattern_redaction_format: PATTERN_REDACTION_FORMAT.to_owned(),
            keyword_redaction_tag: KEYWORD_REDACTION_TAG.to_owned(),
            severity_threshold: args.severity_threshold.clone(),
            router,
            image_model: args.image_model.clone(),
            loaded_categories,
            competitor_intent_checker,
            refuse_message_template: templates.refuse,
            reframe_message_template: templates.reframe,
            compiled_patterns,
            blocked_words,
        })
    }
}

fn guardrail_settings(args: &ContentFilterArgs) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(args.name.clone()),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(event_hook_or_default(&args.event_hook)),
        default_on: args.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: args.end_session_after_n_fails,
        on_violation: args.on_violation.map(on_violation_text),
        realtime_violation_message: args.realtime_violation_message.clone(),
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: CONTENT_FILTER_SWITCHES,
    }
}

// LiteLLM passes `event_hook or pre_call`, so an empty string or list takes the default
fn event_hook_or_default(event_hook: &LitellmParamsMode) -> LitellmParamsMode {
    match event_hook {
        LitellmParamsMode::One(name) if name.is_empty() => {
            LitellmParamsMode::One(DEFAULT_EVENT_HOOK.to_owned())
        }
        LitellmParamsMode::Many(names) if names.is_empty() => {
            LitellmParamsMode::One(DEFAULT_EVENT_HOOK.to_owned())
        }
        LitellmParamsMode::One(_) | LitellmParamsMode::Many(_) | LitellmParamsMode::Tagged(_) => {
            event_hook.clone()
        }
    }
}

fn on_violation_text(action: OnViolation) -> String {
    match action {
        OnViolation::Warn => "warn".to_owned(),
        OnViolation::EndSession => "end_session".to_owned(),
        OnViolation::Block => "block".to_owned(),
        OnViolation::Alert => "alert".to_owned(),
    }
}

fn competitor_checker(
    config: Option<&OpaqueParams>,
) -> (Option<CompetitorIntentChecker>, CompetitorTemplates) {
    let Some(config) = config else {
        return (None, CompetitorTemplates::default());
    };
    match build_competitor(config) {
        Ok((checker, templates)) => (Some(checker), templates),
        Err(error) => {
            tracing::warn!(
                "ContentFilterGuardrail: failed to init competitor intent checker: {error}"
            );
            (None, CompetitorTemplates::default())
        }
    }
}

fn build_competitor(
    config: &OpaqueParams,
) -> Result<(CompetitorIntentChecker, CompetitorTemplates), CompetitorBuildError> {
    let raw: CompetitorConfig = serde_json::from_value(serde_json::to_value(config)?)?;
    let settings = raw.settings();
    let templates = CompetitorTemplates {
        refuse: raw.refuse_message_template.clone(),
        reframe: raw.reframe_message_template.clone(),
    };
    if raw.competitor_intent_type.as_deref() == Some(GENERIC_COMPETITOR) {
        let checker = BaseCompetitorIntentChecker::new(&settings)?;
        return Ok((CompetitorIntentChecker::Generic(checker), templates));
    }
    let airline = AirlineSettings {
        base: settings,
        other_meaning_signals: raw.other_meaning_signals.clone().unwrap_or_default(),
        competitor_signals: raw.competitor_signals.clone().unwrap_or_default(),
        other_meaning_anchors: raw.other_meaning_anchors.clone().unwrap_or_default(),
        explicit_competitor_marker: raw.explicit_competitor_marker.clone(),
        explicit_other_meaning_marker: raw.explicit_other_meaning_marker.clone(),
    };
    let checker = AirlineCompetitorIntentChecker::new(&airline)?;
    Ok((CompetitorIntentChecker::Airline(checker), templates))
}

fn guardrail_categories(configs: Option<&[ContentFilterCategoryConfig]>) -> CategoryLoadResult {
    let Some(configs) = configs.filter(|list| !list.is_empty()) else {
        return CategoryLoadResult::default();
    };
    // LiteLLM's roots are install-time constants, so a missing asset tree skips categories instead of raising
    let roots = match data_roots() {
        Ok(roots) => Vec::from(roots),
        Err(error) => {
            tracing::warn!("content filter data roots are not installed: {error}");
            Vec::new()
        }
    };
    let data_dir = roots
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from(DATA_DIR));
    load_categories(configs, &roots, &data_dir, &CategorySettings::default())
}

fn compile_patterns(
    configs: &[ContentFilterPattern],
) -> Result<Vec<CompiledPattern>, ContentFilterBuildError> {
    let mut compiled = Vec::with_capacity(configs.len());
    for config in configs {
        compiled.push(compile(config)?);
    }
    Ok(compiled)
}

fn warn_on_during_call_mask(base: &GuardrailBase, patterns: &[CompiledPattern]) {
    let during_call = match &base.settings.event_hook {
        Some(LitellmParamsMode::One(name)) => name == DURING_CALL,
        Some(LitellmParamsMode::Many(_) | LitellmParamsMode::Tagged(_)) | None => false,
    };
    if !during_call
        || !patterns
            .iter()
            .any(|pattern| pattern.action == ContentFilterAction::Mask)
    {
        return;
    }
    let name = base.settings.guardrail_name.as_deref().unwrap_or("None");
    tracing::warn!(
        "ContentFilterGuardrail '{name}': 'during_call' mode with 'MASK' action is unstable due to race conditions. Use 'pre_call' mode for reliable request masking."
    );
}

fn blocked_word_map(
    words: &[BlockedWord],
    file: Option<&str>,
) -> Result<IndexMap<String, (ContentFilterAction, Option<String>)>, ContentFilterBuildError> {
    let mut map = IndexMap::new();
    for word in words {
        insert_blocked_word(&mut map, word);
    }
    if let Some(path) = file.filter(|path| !path.is_empty()) {
        for word in load_blocked_words(Path::new(path))? {
            insert_blocked_word(&mut map, &word);
        }
    }
    Ok(map)
}

fn insert_blocked_word(
    map: &mut IndexMap<String, (ContentFilterAction, Option<String>)>,
    word: &BlockedWord,
) {
    map.insert(
        word.keyword.to_lowercase(),
        (word.action, word.description.clone()),
    );
}
