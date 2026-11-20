use betterlite_core::api::vector_search_execution::config::Params;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::api::vector_store_search::error::VectorStoreSearchError;
use betterlite_core::api::vector_store_search::service::{
    SdkDeps, SearchCall, search as search_vector_store,
};
use betterlite_models::decisions::Json;

use super::super::support::{Source, search_optionals};
use super::super::{Run, SearchArgs};

pub(super) async fn search_run(
    run: &Run,
    args: &SearchArgs,
    deps: &SdkDeps<'_, Source>,
    kwargs: &Params,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError> {
    let optional = search_optionals(args);
    let call = SearchCall {
        vector_store_id: &args.vector_store_id,
        query: &args.query,
        optional: &optional,
        kwargs,
        extra_headers: run.extra_headers.as_ref(),
        extra_body: run.extra_body.as_ref(),
        timeout: run.timeout_seconds,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        executor: None,
        entry: run.mode,
    };
    search_vector_store(deps, &call, log).await
}
