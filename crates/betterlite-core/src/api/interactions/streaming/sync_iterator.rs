use betterlite_interactions::types::response::InteractionsStreamChunk;

use super::base::InteractionsStreamBase;
use super::error::{InteractionStreamError, SyncStreamError};
use super::logging::InteractionsStreamLogging;
use super::records::{
    HiddenParamsInput, InteractionsStreamSetup, InteractionsSuccessCall, hidden_params,
};
use super::response::InteractionsSyncResponse;
use crate::api::interactions::contract::config::BaseInteractionsAPIConfig;

pub struct SyncInteractionsStreamIterator<'base, Response, Config, Logging>
where
    Response: InteractionsSyncResponse,
    Config: BaseInteractionsAPIConfig,
    Logging: InteractionsStreamLogging,
{
    pub lines: Response::Lines,
    pub base: InteractionsStreamBase<'base, Config, Logging>,
}

impl<'base, Response, Config, Logging>
    SyncInteractionsStreamIterator<'base, Response, Config, Logging>
where
    Response: InteractionsSyncResponse,
    Config: BaseInteractionsAPIConfig,
    Logging: InteractionsStreamLogging,
{
    pub fn new(
        response: Response,
        setup: InteractionsStreamSetup<'base, Config, Logging>,
    ) -> Result<Self, SyncStreamError<Response, Config>> {
        let InteractionsStreamSetup {
            model,
            config,
            logging,
            litellm_metadata,
            custom_llm_provider,
            api_base,
        } = setup;
        let headers = response.headers();
        let hidden_params = hidden_params(HiddenParamsInput {
            headers: &headers,
            model,
            logging: &mut *logging,
            metadata: litellm_metadata,
            api_base,
        })?;
        let start_time = logging.clock().now();
        let lines = response.iter_lines();
        Ok(Self {
            lines,
            base: InteractionsStreamBase {
                model: model.map(str::to_owned),
                custom_llm_provider: custom_llm_provider.map(str::to_owned),
                config,
                logging,
                hidden_params,
                start_time,
                completed_response: None,
                finished: false,
            },
        })
    }

    pub fn next_chunk(
        &mut self,
    ) -> Option<Result<InteractionsStreamChunk, SyncStreamError<Response, Config>>> {
        if self.base.finished {
            return None;
        }
        loop {
            let line = match self.lines.next() {
                Some(Ok(line)) => line,
                Some(Err(source)) => {
                    self.base.finished = true;
                    return Some(Err(InteractionStreamError::Source { source }));
                }
                None => {
                    self.base.finished = true;
                    return None;
                }
            };
            let outcome = match self.base.process_chunk(&line) {
                Ok(outcome) => outcome,
                Err(source) => return Some(Err(InteractionStreamError::Transform { source })),
            };
            if outcome.completed {
                self.dispatch_completed();
            }
            if self.base.finished {
                return None;
            }
            if let Some(event) = outcome.event {
                return Some(Ok(event));
            }
        }
    }

    fn dispatch_completed(&mut self) {
        let Some(response) = self.base.completed_response.clone() else {
            return;
        };
        let async_call = InteractionsSuccessCall {
            response: response.clone(),
            start_time: self.base.start_time,
            end_time: self.base.logging.clock().now(),
            cache_hit: None,
            prefer_async_handlers: false,
        };
        self.base.logging.run_async_success_handler(async_call);
        let sync_call = InteractionsSuccessCall {
            response,
            start_time: self.base.start_time,
            end_time: self.base.logging.clock().now(),
            cache_hit: None,
            prefer_async_handlers: false,
        };
        self.base.logging.submit_success_handler(sync_call);
    }
}
