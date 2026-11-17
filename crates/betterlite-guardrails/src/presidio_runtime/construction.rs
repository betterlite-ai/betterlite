use std::collections::{BTreeMap, HashMap};
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use reqwest::Client;

use super::PresidioGuardParams;
use super::error::PresidioRuntimeError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_initializers::presidio::PresidioArgs;
use crate::presidio_actions::validate_environment;
use crate::presidio_actions::{AnonymizerSettings, PresidioAnonymizer, PresidioEndpoints};
use crate::presidio_analysis::split::coerce_analyze_chunk_size;
use crate::presidio_analysis::wire::MockRedactedText;
use crate::presidio_analysis::{AnalyzerSettings, DEFAULT_ANALYZE_TIMEOUT, PresidioAnalyzer};

const LOGGING_ONLY_HOOK: &str = "logging_only";
const POST_CALL_HOOK: &str = "post_call";
const DEFAULT_LANGUAGE: &str = "en";

pub(super) struct Built {
    pub(super) analyzer: PresidioAnalyzer,
    pub(super) anonymizer: PresidioAnonymizer,
    pub(super) pii_entities_config: BTreeMap<String, PiiAction>,
}

struct Resolved {
    endpoints: PresidioEndpoints,
    ad_hoc_recognizers: Option<Vec<GuardrailValue>>,
    timeout: Duration,
    pii_entities_config: BTreeMap<String, PiiAction>,
}

pub(super) fn build(
    args: &PresidioArgs,
    params: &PresidioGuardParams<'_>,
    base: &mut GuardrailBase,
    client: Client,
) -> Result<Built, PresidioRuntimeError> {
    let output_parse_pii = args.output_parse_pii.unwrap_or(false);
    apply_event_hooks(base, args, params);
    let resolved = resolve(args, params)?;
    Ok(Built {
        analyzer: PresidioAnalyzer {
            settings: analyzer_settings(args, &resolved, output_parse_pii)?,
            client: client.clone(),
            chunk_semaphores: Mutex::new(HashMap::new()),
        },
        anonymizer: PresidioAnonymizer {
            settings: AnonymizerSettings {
                anonymizer_api_base: resolved.endpoints.anonymizer_api_base,
                timeout: resolved.timeout,
            },
            client,
            clock: SystemTime::now,
        },
        pii_entities_config: resolved.pii_entities_config,
    })
}

fn resolve(
    args: &PresidioArgs,
    params: &PresidioGuardParams<'_>,
) -> Result<Resolved, PresidioRuntimeError> {
    // LiteLLM reads the recognizer file before it checks the API bases, so a bad path wins over a missing base
    let ad_hoc_recognizers = recognizers(args.presidio_ad_hoc_recognizers.as_deref())?;
    let analyzer_argument = args
        .presidio_analyzer_api_base
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(params.defaults.analyzer_api_base.as_deref());
    let anonymizer_argument = args
        .presidio_anonymizer_api_base
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(params.defaults.anonymizer_api_base.as_deref());
    Ok(Resolved {
        endpoints: validate_environment(analyzer_argument, anonymizer_argument, |_variable| None)?,
        ad_hoc_recognizers,
        timeout: timeout(args.timeout)?,
        pii_entities_config: args.pii_entities_config.clone().unwrap_or_default(),
    })
}

fn analyzer_settings(
    args: &PresidioArgs,
    resolved: &Resolved,
    output_parse_pii: bool,
) -> Result<AnalyzerSettings, PresidioRuntimeError> {
    Ok(AnalyzerSettings {
        guardrail_name: args.name.clone(),
        analyzer_api_base: resolved.endpoints.analyzer_api_base.clone(),
        language: args
            .presidio_language
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_LANGUAGE)
            .to_owned(),
        ad_hoc_recognizers: resolved.ad_hoc_recognizers.clone(),
        pii_entities: resolved.pii_entities_config.keys().cloned().collect(),
        output_parse_pii,
        apply_to_output: args.apply_to_output,
        score_thresholds: args.presidio_score_thresholds.clone().unwrap_or_default(),
        entities_deny_list: args.presidio_entities_deny_list.clone().unwrap_or_default(),
        chunk_size_bytes: coerce_analyze_chunk_size(
            args.analyze_chunk_size_bytes
                .map(|size| i64::try_from(size).unwrap_or(i64::MAX)),
        ),
        timeout: resolved.timeout,
        mock_redacted_text: mock_redacted_text(args.mock_redacted_text.as_ref())?,
    })
}

fn apply_event_hooks(
    base: &mut GuardrailBase,
    args: &PresidioArgs,
    params: &PresidioGuardParams<'_>,
) {
    if params.logging_only {
        base.settings.event_hook = Some(LitellmParamsMode::One(LOGGING_ONLY_HOOK.to_owned()));
        return;
    }
    let output_enabled = args.output_parse_pii.unwrap_or(false) || args.apply_to_output;
    // Only the legacy roleless callback widens its hook; the callbacks B3681 builds already carry theirs
    if params.callback_role.is_some() || !output_enabled {
        return;
    }
    base.settings.event_hook = widened(base.settings.event_hook.take());
}

fn widened(hook: Option<LitellmParamsMode>) -> Option<LitellmParamsMode> {
    match hook {
        Some(LitellmParamsMode::One(name)) if name != POST_CALL_HOOK => {
            Some(LitellmParamsMode::Many(vec![
                name,
                POST_CALL_HOOK.to_owned(),
            ]))
        }
        Some(LitellmParamsMode::Many(mut names))
            if !names.iter().any(|name| name == POST_CALL_HOOK) =>
        {
            names.push(POST_CALL_HOOK.to_owned());
            Some(LitellmParamsMode::Many(names))
        }
        other => other,
    }
}

fn recognizers(path: Option<&str>) -> Result<Option<Vec<GuardrailValue>>, PresidioRuntimeError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path).map_err(|source| read_error(path, source))?;
    // A JSON null reads as no recognizers, which LiteLLM leaves out of the analyze payload
    serde_json::from_str::<Option<Vec<GuardrailValue>>>(&text).map_err(|source| {
        PresidioRuntimeError::Decode {
            path: path.to_owned(),
            source,
        }
    })
}

fn read_error(path: &str, source: IoError) -> PresidioRuntimeError {
    if source.kind() == ErrorKind::NotFound {
        return PresidioRuntimeError::FileNotFound {
            path: path.to_owned(),
        };
    }
    PresidioRuntimeError::Read {
        path: path.to_owned(),
        source,
    }
}

fn timeout(configured: Option<f64>) -> Result<Duration, PresidioRuntimeError> {
    match configured {
        None => Ok(DEFAULT_ANALYZE_TIMEOUT),
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|source| PresidioRuntimeError::Timeout { seconds, source }),
    }
}

fn mock_redacted_text(
    configured: Option<&BTreeMap<String, GuardrailValue>>,
) -> Result<Option<MockRedactedText>, PresidioRuntimeError> {
    let Some(fields) = configured else {
        return Ok(None);
    };
    let encoded = serde_json::to_string(fields)
        .map_err(|source| PresidioRuntimeError::MockRedactedText { source })?;
    serde_json::from_str(&encoded)
        .map(Some)
        .map_err(|source| PresidioRuntimeError::MockRedactedText { source })
}
