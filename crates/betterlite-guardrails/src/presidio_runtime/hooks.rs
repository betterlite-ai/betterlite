use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use tokio::sync::mpsc::{Receiver, Sender};

use super::PresidioGuard;
use super::data::{store_request_data, to_request_data};
use crate::cato::error::ModelStreamError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_selection::gate::GuardrailSelection;
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{
    PresidioPerRequestConfig, RequestData, presidio_settings_from_request,
};
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::messages::{
    TextTask, apply_masked_texts, collect_text_tasks, request_messages, store_messages,
};
use crate::presidio_response::wire::ResponseBody;
use crate::presidio_response::{HookContext, logging, post_call};
use crate::presidio_streams::assembly::StreamAssembly;
use crate::presidio_streams::{StreamEvent, StreamRun, hook};

type Data = BTreeMap<String, GuardrailValue>;

impl PresidioGuard {
    pub async fn pre_call(
        &self,
        data: &mut Data,
        _call_type: &str,
    ) -> Result<(), GuardrailFailure> {
        if !self
            .base
            .should_run_guardrail(data, GuardrailEventHooks::PreCall, &BTreeSet::new())?
        {
            return Ok(());
        }
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        let outcome = self.mask_request(&mut request, &dynamic_params).await;
        // LiteLLM keeps the record a failed check appended, so the converted data goes back either way
        store_request_data(data, &request)?;
        outcome.map_err(GuardrailFailure::from)
    }

    pub async fn post_call_success(
        &self,
        data: &mut Data,
        response: &mut ResponseBody,
    ) -> Result<(), GuardrailFailure> {
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        post_call::async_post_call_success_hook(
            self.context(&dynamic_params),
            &mut request,
            response,
        )
        .await?;
        store_request_data(data, &request)?;
        Ok(())
    }

    pub fn logging_hook(
        &self,
        data: &mut Data,
        response: &mut ResponseBody,
        call_type: &str,
    ) -> Result<(), GuardrailFailure> {
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        logging::logging_hook(
            self.context(&dynamic_params),
            &mut request,
            response,
            call_type,
        )?;
        store_request_data(data, &request)?;
        Ok(())
    }

    pub async fn async_logging_hook(
        &self,
        data: &mut Data,
        response: &mut ResponseBody,
        call_type: &str,
    ) -> Result<(), GuardrailFailure> {
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        logging::async_logging_hook(
            self.context(&dynamic_params),
            &mut request,
            response,
            call_type,
        )
        .await?;
        store_request_data(data, &request)?;
        Ok(())
    }

    pub async fn streaming_iterator<A>(
        &self,
        data: &mut Data,
        assembly: &A,
        input: &mut Receiver<Result<StreamEvent, ModelStreamError>>,
        out: &Sender<StreamEvent>,
    ) -> Result<(), GuardrailFailure>
    where
        A: StreamAssembly,
    {
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        let outcome = hook::streaming_iterator_hook(
            &mut StreamRun {
                context: self.context(&dynamic_params),
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

    fn context<'context>(
        &'context self,
        dynamic_params: &'context BTreeMap<String, GuardrailValue>,
    ) -> HookContext<'context> {
        HookContext {
            analyzer: &self.analyzer,
            anonymizer: &self.anonymizer,
            pii_entities_config: &self.pii_entities_config,
            dynamic_params,
            global_output_parse_pii: self.global_output_parse_pii,
        }
    }

    async fn mask_request(
        &self,
        request: &mut RequestData,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
    ) -> Result<(), PresidioResponseError> {
        let presidio_config = presidio_settings_from_request(request)?;
        let Some(mut messages) = request_messages(request)? else {
            return Ok(());
        };
        let tasks = collect_text_tasks(&messages);
        let masked = self
            .mask_texts(&tasks, presidio_config.as_ref(), dynamic_params, request)
            .await?;
        apply_masked_texts(&mut messages, &tasks, &masked);
        store_messages(request, &messages)
    }

    async fn mask_texts(
        &self,
        tasks: &[TextTask],
        presidio_config: Option<&PresidioPerRequestConfig>,
        dynamic_params: &BTreeMap<String, GuardrailValue>,
        request: &mut RequestData,
    ) -> Result<Vec<Option<String>>, PresidioActionsError> {
        let mut masked = Vec::with_capacity(tasks.len());
        let mut failure = None;
        for task in tasks {
            let outcome = self
                .anonymizer
                .check_pii(
                    &self.analyzer,
                    CheckPiiCall {
                        text: &task.text,
                        output_parse_pii: self.analyzer.settings.output_parse_pii,
                        presidio_config,
                        dynamic_params,
                        pii_entities_config: &self.pii_entities_config,
                        request_data: &mut *request,
                    },
                )
                .await;
            match outcome {
                Ok(text) => masked.push(Some(text)),
                Err(error) => {
                    failure.get_or_insert(error);
                    masked.push(None);
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(masked)
    }
}
