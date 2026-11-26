use std::sync::atomic::AtomicBool;

use betterlite_config::sections::batch_limit_descriptors::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_input_accounting::input::BatchInputAccountingInput;
use betterlite_core::api::batch_input_accounting::service::BatchInputAccounting;
use betterlite_core::api::batch_limit_descriptors::BatchLimitDescriptors;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

use self::input::Input;
use self::report::{failure, record_report};
use self::scripted::{
    LiteLlmLimiter, NoCredentials, ScriptedAccess, ScriptedCounter, ScriptedFiles,
};

mod input;
mod report;
mod scripted;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let files = ScriptedFiles::new(input.content.clone(), input.managed.clone());
        let access = ScriptedAccess::new(input.access.clone());
        let counter = ScriptedCounter::new(input.tokens.clone());
        let service: BatchLimitDescriptors<LiteLlmLimiter, NoCredentials> = BatchLimitDescriptors {
            settings: Settings::default(),
            limiter: LiteLlmLimiter,
            credentials: None,
            limits: ApiLimits::default(),
            warned_unsupported_model_skip: AtomicBool::new(false),
        };
        let accounting =
            BatchInputAccounting::new(&service, &files, &access, &counter, ApiLimits::default());
        let record = BatchInputAccountingInput {
            file_id: input.file_id.clone(),
            custom_llm_provider: input.custom_llm_provider,
            caller: input.caller.clone(),
            data: input.data.clone(),
            descriptors: input.descriptors.clone(),
        };
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let outcome = runtime.block_on(accounting.run(&record));
        record_report(outcome, &files, &counter)
    }
}
