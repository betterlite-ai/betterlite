use tokio::runtime::Builder;

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::HookContext;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::logging::async_logging_hook;
use crate::presidio_response::wire::ResponseBody;

pub(super) fn run_on_new_runtime(
    context: HookContext<'_>,
    request: &mut RequestData,
    response: &mut ResponseBody,
    call_type: &str,
) -> Result<(), PresidioResponseError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| PresidioResponseError::Runtime { source })?;
    runtime.block_on(async_logging_hook(context, request, response, call_type))
}
