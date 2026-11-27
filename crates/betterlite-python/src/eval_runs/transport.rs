use betterlite_core::api::eval_http::exchange::{EvalConnection, EvalLogging};
use betterlite_core::api::eval_run_create_list::error::EvalRunCallError;
use betterlite_core::api::eval_run_create_list::providers::{
    CreateRunCall, ListRunsCall, PendingCreateRun, PendingListRuns, RunTransport,
};
use betterlite_core::api::eval_run_http::{create_run_handler, list_runs_handler};
use betterlite_http::proxy::ProxiedClient;

// B9371's two run handlers behind B3424's transport seam, over the session's pooled client.
pub(super) struct HttpRunTransport<'client> {
    pub(super) client: &'client ProxiedClient,
}

impl RunTransport for HttpRunTransport<'_> {
    fn create_run<'call>(
        &'call self,
        log: EvalLogging<'call, '_>,
        call: CreateRunCall<'call>,
    ) -> PendingCreateRun<'call> {
        Box::pin(async move {
            let connection = EvalConnection {
                config: call.config,
                client: self.client,
                url: &call.url,
                headers: &call.headers,
            };
            let mut logging = log;
            create_run_handler(&connection, &mut logging, Some(call.timeout), &call.body)
                .await
                .map_err(EvalRunCallError::Transport)
        })
    }

    // LiteLLM's list GET sends no timeout, so the handler leaves the client's own deadline in place.
    fn list_runs<'call>(
        &'call self,
        log: EvalLogging<'call, '_>,
        call: ListRunsCall<'call>,
    ) -> PendingListRuns<'call> {
        Box::pin(async move {
            let connection = EvalConnection {
                config: call.config,
                client: self.client,
                url: &call.url,
                headers: &call.headers,
            };
            let mut logging = log;
            list_runs_handler(&connection, &mut logging, &call.query)
                .await
                .map_err(EvalRunCallError::Transport)
        })
    }
}
