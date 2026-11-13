use betterlite_logging::logging_success::ports::SuccessClock;

use super::records::InteractionsSuccessCall;
use crate::api::interactions::contract::records::InteractionsLogging;

pub trait InteractionsStreamLogging {
    fn logging(&mut self) -> &mut InteractionsLogging;

    fn clock(&self) -> &dyn SuccessClock;

    // LiteLLM schedules this dispatch on the event loop and returns; the logging side owns the task it starts
    fn dispatch_success_handlers(&self, call: InteractionsSuccessCall);

    // LiteLLM's run_async_function runs the async handler to completion from the sync iterator
    fn run_async_success_handler(&self, call: InteractionsSuccessCall);

    // LiteLLM submits the sync handler to the logging executor
    fn submit_success_handler(&self, call: InteractionsSuccessCall);
}
