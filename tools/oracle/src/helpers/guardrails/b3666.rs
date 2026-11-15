mod hooks;
mod scan;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_deployment::markers::{PreCallMarkers as _, PreCallToken};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

type Data = BTreeMap<String, GuardrailValue>;

// The oracle swaps LiteLLM's per-process token for this one on both sides
const FIXED_TOKEN: &str = "fixed-token";

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Settings {
    guardrail_name: Option<String>,
    event_hook: Option<LitellmParamsMode>,
    default_on: GuardrailFlag,
    only_scan_new_messages: GuardrailFlag,
    mask_request_content: GuardrailFlag,
    mask_response_content: GuardrailFlag,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ScanFilter {
        #[serde(flatten)]
        settings: Settings,
        texts: Option<Vec<String>>,
        cached: Vec<String>,
        data: Data,
    },
    ScanMark {
        #[serde(flatten)]
        settings: Settings,
        texts: Option<Vec<String>>,
        existing: Vec<String>,
        data: Data,
    },
    MarkPreCall {
        #[serde(flatten)]
        settings: Settings,
        data: Data,
    },
    AlreadyRan {
        #[serde(flatten)]
        settings: Settings,
        data: Data,
    },
    StampedKeyFields {
        #[serde(flatten)]
        settings: Settings,
        data: Data,
    },
    DeploymentPre {
        #[serde(flatten)]
        settings: Settings,
        kwargs: Data,
        call_type: Option<CallType>,
        suppressed: BTreeSet<String>,
    },
    DeploymentPost {
        #[serde(flatten)]
        settings: Settings,
        request_data: Data,
        suppressed: BTreeSet<String>,
    },
}

fn guardrail_of(settings: &Settings) -> GuardrailBase {
    let mut base = GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    };
    base.settings
        .guardrail_name
        .clone_from(&settings.guardrail_name);
    base.settings.event_hook.clone_from(&settings.event_hook);
    base.settings.default_on = settings.default_on;
    base.settings.only_scan_new_messages = settings.only_scan_new_messages;
    base.settings.mask_request_content = settings.mask_request_content;
    base.settings.mask_response_content = settings.mask_response_content;
    base
}

async fn dispatch(input: &Input) -> Result<Value, Box<HelperError>> {
    match input {
        Input::ScanFilter {
            settings,
            texts,
            cached,
            data,
        } => Ok(scan::scan(settings, texts.as_deref(), cached, data, false).await),
        Input::ScanMark {
            settings,
            texts,
            existing,
            data,
        } => Ok(scan::scan(settings, texts.as_deref(), existing, data, true).await),
        Input::MarkPreCall { settings, data } => {
            let mut scratch = data.clone();
            let token = PreCallToken(FIXED_TOKEN.to_owned());
            guardrail_of(settings).mark_pre_call_hook_ran(&token, &mut scratch);
            Ok(json!(scratch))
        }
        Input::AlreadyRan { settings, data } => Ok(Value::Bool(
            hooks::probed_keys(settings, data).await?.is_empty(),
        )),
        Input::StampedKeyFields { settings, data } => hooks::stamped(settings, data).await,
        Input::DeploymentPre {
            settings,
            kwargs,
            call_type,
            suppressed,
        } => hooks::pre_hook(settings, kwargs, *call_type, suppressed).await,
        Input::DeploymentPost {
            settings,
            request_data,
            suppressed,
        } => hooks::post_hook(settings, request_data, suppressed).await,
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .or_value_error()?;
        let result = runtime.block_on(dispatch(input))?;
        Ok(json!({ "result": result }))
    }
}
