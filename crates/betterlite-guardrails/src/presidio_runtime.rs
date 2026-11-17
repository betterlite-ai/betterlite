pub mod error;

mod construction;
mod contract;
mod data;
mod hooks;
mod tests;

use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;
use reqwest::Client;
use tokio::sync::mpsc::{Receiver, Sender};

use self::error::{INTERNAL_STATUS, PresidioRuntimeError};
use self::hooks::{AsyncHook, Data};
use crate::cato::error::ModelStreamError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::presidio::{PresidioArgs, PresidioRole};
use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_analysis::PresidioAnalyzer;
use crate::presidio_response::wire::ResponseBody;
use crate::presidio_streams::StreamEvent;
use crate::presidio_streams::assembly::StreamAssembly;
use crate::presidio_streams::error::PresidioStreamError;

type Body = ResponseBody;
type Outcome = Result<(), GuardrailFailure>;

// The two bases LiteLLM reads through get_secret; B8666 resolves them with B8500's lookup and hands them in
#[derive(Debug, Clone, Default)]
pub struct PresidioDefaults {
    pub analyzer_api_base: Option<String>,
    pub anonymizer_api_base: Option<String>,
}

pub struct PresidioGuardParams<'settings> {
    pub defaults: PresidioDefaults,
    pub settings: &'settings RuntimeSettings,
    pub logging_only: bool,
    // every callback B3681 builds has a role; only the roleless legacy callback leaves this None
    pub callback_role: Option<PresidioRole>,
}

pub struct PresidioGuard {
    base: GuardrailBase,
    analyzer: PresidioAnalyzer,
    anonymizer: PresidioAnonymizer,
    pii_entities_config: BTreeMap<String, PiiAction>,
    // litellm_settings.output_parse_pii, which B3806's post-call hook ORs with the instance setting
    global_output_parse_pii: bool,
    callback_role: Option<PresidioRole>,
}

impl PresidioGuard {
    pub fn new(
        args: &PresidioArgs,
        params: &PresidioGuardParams<'_>,
        mut base: GuardrailBase,
        client: Client,
    ) -> Result<Self, PresidioRuntimeError> {
        let built = construction::build(args, params, &mut base, client)?;
        Ok(Self {
            base,
            analyzer: built.analyzer,
            anonymizer: built.anonymizer,
            pii_entities_config: built.pii_entities_config,
            global_output_parse_pii: params.settings.output_parse_pii,
            callback_role: params.callback_role,
        })
    }

    pub async fn pre_call(&self, data: &mut Data, call_type: &str) -> Outcome {
        hooks::pre_call(self, data, call_type).await
    }

    pub async fn post_call_success(&self, data: &mut Data, response: &mut Body) -> Outcome {
        hooks::run_async(self, data, AsyncHook::PostCall(response)).await
    }

    pub fn logging_hook(&self, data: &mut Data, response: &mut Body, call_type: &str) -> Outcome {
        hooks::logging_hook(self, data, response, call_type)
    }

    pub async fn async_logging_hook(
        &self,
        data: &mut Data,
        response: &mut Body,
        call_type: &str,
    ) -> Outcome {
        hooks::run_async(
            self,
            data,
            AsyncHook::AsyncLogging {
                response,
                call_type,
            },
        )
        .await
    }

    pub async fn streaming_iterator<Assembly>(
        &self,
        data: &mut Data,
        assembly: &Assembly,
        input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
        out: &Sender<StreamEvent>,
    ) -> Outcome
    where
        Assembly: StreamAssembly,
    {
        hooks::streaming_iterator(self, data, assembly, input, out).await
    }
}

impl<Failure> From<PresidioStreamError<Failure>> for GuardrailFailure
where
    Failure: StdError + 'static,
{
    fn from(error: PresidioStreamError<Failure>) -> Self {
        match error {
            PresidioStreamError::Actions(actions) => Self::from(actions),
            PresidioStreamError::Response(response) => Self::from(response),
            other @ (PresidioStreamError::OverBudget { .. }
            | PresidioStreamError::Upstream { .. }
            | PresidioStreamError::Assembly { .. }
            | PresidioStreamError::Json { .. }
            | PresidioStreamError::ConsumerClosed) => Self::Http {
                status: INTERNAL_STATUS,
                detail: other.to_string(),
            },
        }
    }
}
