pub mod error;

mod construction;
mod contract;
mod data;
mod hooks;
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;
use reqwest::Client;
use tokio::sync::mpsc::{Receiver, Sender};

use self::data::{store_request_data, to_request_data};
use self::error::{INTERNAL_STATUS, PresidioRuntimeError};
use self::hooks::{Data, context, mask_request};
use crate::cato::error::ModelStreamError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::presidio::{PresidioArgs, PresidioRole};
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_analysis::PresidioAnalyzer;
use crate::presidio_response::wire::ResponseBody;
use crate::presidio_response::{logging, post_call};
use crate::presidio_streams::StreamEvent;
use crate::presidio_streams::assembly::StreamAssembly;
use crate::presidio_streams::error::PresidioStreamError;
use crate::presidio_streams::{StreamRun, hook};

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

// The response hooks all convert the request data, run one B3806 function and write the data back, so the class defines them through one skeleton
macro_rules! response_hook {
    (sync $name:ident, $call:path) => {
        pub fn $name(&self, data: &mut Data, response: &mut Body, call_type: &str) -> Outcome {
            let dynamic_params = self.base.dynamic_request_body_params(data)?;
            let mut request = to_request_data(data)?;
            $call(context(self, &dynamic_params), &mut request, response, call_type)?;
            store_request_data(data, &request)?;
            Ok(())
        }
    };
    ($name:ident $(+ $call_type:ident)?, $call:path) => {
        pub async fn $name(
            &self,
            data: &mut Data,
            response: &mut Body,
            $($call_type: &str)?
        ) -> Outcome {
            let dynamic_params = self.base.dynamic_request_body_params(data)?;
            let mut request = to_request_data(data)?;
            $call(
                context(self, &dynamic_params),
                &mut request,
                response $(, $call_type)?,
            )
            .await?;
            store_request_data(data, &request)?;
            Ok(())
        }
    };
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

    pub async fn pre_call(&self, data: &mut Data, _call_type: &str) -> Outcome {
        if !self
            .base
            .should_run_guardrail(data, GuardrailEventHooks::PreCall, &BTreeSet::new())?
        {
            return Ok(());
        }
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        let outcome = mask_request(self, &mut request, &dynamic_params).await;
        // LiteLLM keeps the record a failed check appended, so the converted data goes back either way
        store_request_data(data, &request)?;
        outcome.map_err(GuardrailFailure::from)
    }

    response_hook!(post_call_success, post_call::async_post_call_success_hook);
    response_hook!(async_logging_hook + call_type, logging::async_logging_hook);
    response_hook!(sync logging_hook, logging::logging_hook);

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
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        let outcome = hook::streaming_iterator_hook(
            &mut StreamRun {
                context: context(self, &dynamic_params),
                request_data: &mut request,
                assembly,
                out,
            },
            input,
        )
        .await;
        store_request_data(data, &request)?;
        outcome.map_err(GuardrailFailure::from)
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
