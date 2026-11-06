use std::sync::Mutex;

use betterlite_http::proxy::base_builder;
use tokio::runtime::Builder;

use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_actions::request_data::RequestData;
use crate::presidio_analysis::PresidioAnalyzer;
use crate::presidio_response::HookContext;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::logging::async_logging_hook;
use crate::presidio_response::wire::ResponseBody;

// The hook owns this client and its pool, so the connection tasks it starts die with the hook's throwaway runtime instead of poisoning the clients the server's long-lived runtime shares.
pub(super) fn run_with_owned_client(
    context: HookContext<'_>,
    request: &mut RequestData,
    response: &mut ResponseBody,
    call_type: &str,
) -> Result<(), PresidioResponseError> {
    let client = base_builder()
        .build()
        .map_err(|source| PresidioResponseError::Client { source })?;
    let analyzer = PresidioAnalyzer {
        settings: context.analyzer.settings.clone(),
        client: client.clone(),
        chunk_semaphores: Mutex::default(),
    };
    let anonymizer = PresidioAnonymizer {
        settings: context.anonymizer.settings.clone(),
        client,
        clock: context.anonymizer.clock,
    };
    let owned = HookContext {
        analyzer: &analyzer,
        anonymizer: &anonymizer,
        pii_entities_config: context.pii_entities_config,
        dynamic_params: context.dynamic_params,
        global_output_parse_pii: context.global_output_parse_pii,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| PresidioResponseError::Runtime { source })?;
    runtime.block_on(async_logging_hook(owned, request, response, call_type))
}
