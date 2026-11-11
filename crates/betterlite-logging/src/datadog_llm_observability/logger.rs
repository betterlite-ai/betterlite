use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use tokio_util::task::TaskTracker;

use crate::batch::logger::BatchQueue;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::datadog::apm::Tracer;
use crate::datadog_llm_metrics::token_counting::ToolTokenCounter;
use crate::datadog_llm_obs::span_records::LLMObsPayload;
use crate::datadog_llm_obs::values::Object;
use crate::datadog_logging::logger::IdGenerator;
use crate::datadog_logging::transport::Transport;

use super::error::DatadogLlmObsError;
use super::flush;
use super::settings::{DatadogLlmObsInit, DatadogLlmObsSettings};
use super::span::{self, SpanContext, SpanRequest};

pub struct DatadogLlmObsDeps {
    pub transport: Arc<dyn Transport>,
    pub tracer: Arc<dyn Tracer + Send + Sync>,
    pub ids: IdGenerator,
    // the server owns the flush timer's task; without a tracker only explicit flushes run
    pub tasks: Option<TaskTracker>,
}

pub struct DatadogLlmObsLogger<Counter: ToolTokenCounter> {
    pub settings: DatadogLlmObsSettings,
    pub queue: BatchQueue<LLMObsPayload>,
    pub transport: Arc<dyn Transport>,
    pub tracer: Arc<dyn Tracer + Send + Sync>,
    pub ids: IdGenerator,
    pub counter: Arc<Counter>,
}

impl<Counter> DatadogLlmObsLogger<Counter>
where
    Counter: ToolTokenCounter + Send + Sync + 'static,
{
    pub fn new(
        init: &DatadogLlmObsInit,
        environment: &dyn EnvironmentLookup,
        counter: Arc<Counter>,
        deps: DatadogLlmObsDeps,
    ) -> Result<Arc<Self>, DatadogLlmObsError> {
        let settings = DatadogLlmObsSettings::resolve(init, environment)?;
        let logger = Arc::new(Self {
            queue: BatchQueue::new(settings.batch),
            settings,
            transport: deps.transport,
            tracer: deps.tracer,
            ids: deps.ids,
            counter,
        });
        match deps.tasks {
            Some(tasks) => {
                let periodic = Arc::clone(&logger);
                let tracker = tasks.clone();
                let _task =
                    tasks.spawn(async move { flush::periodic_flush(periodic, tracker).await });
            }
            None => {
                tracing::debug!("DataDogLLMObs: no task tracker, the periodic flush did not start");
            }
        }
        Ok(logger)
    }

    async fn log_event(&self, event: &LoggerEvent) {
        let empty = Object::new();
        let metadata = match event.dynamic_params.as_ref() {
            Json::Object(fields) => fields,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &empty,
        };
        let redact = span::logging_is_off(
            &self.settings.options,
            metadata,
            self.settings.global_turn_off,
        );
        let apm_id = span::apm_trace_id(self.tracer.as_ref());
        let request = SpanRequest {
            payload: event.payload.as_ref(),
            request_metadata: metadata,
            redact,
            apm_id: apm_id.as_deref(),
        };
        let context = SpanContext {
            settings: &self.settings,
            counter: self.counter.as_ref(),
            generate_id: &self.ids,
        };
        let span = match span::span_payload(&request, &context).await {
            Ok(span) => span,
            Err(error) => {
                tracing::error!(%error, model = %event.payload.model, "DataDogLLMObs: Error assembling the span");
                return;
            }
        };
        let enqueued = self.queue.enqueue(span);
        tracing::debug!(
            depth = enqueued.depth,
            "DataDogLLMObs: event added to the queue"
        );
        if enqueued.depth >= self.settings.batch.batch_size {
            flush::async_send_batch(self).await;
        }
    }
}

#[async_trait]
impl<Counter> CallbackLogger for DatadogLlmObsLogger<Counter>
where
    Counter: ToolTokenCounter + Send + Sync + 'static,
{
    fn callback_name(&self) -> &'static str {
        "datadog_llm_observability"
    }

    fn turn_off_message_logging(&self) -> bool {
        self.settings.options.turn_off_message_logging
    }

    fn message_logging(&self) -> bool {
        self.settings.options.message_logging
    }

    // LiteLLM returns True here, so the base logger hands this one the unredacted payload
    fn redacts_messages_itself(&self) -> bool {
        true
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_event(&event).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_event(&event).await;
        Ok(())
    }
}
