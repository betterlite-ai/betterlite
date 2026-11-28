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

// The two handlers differ only in their call, the timeout create sends, and the body or query it carries.
macro_rules! run_handler {
    ($method:ident, $call:ident, $pending:ident, $handler:ident, payload $payload:ident $(, timeout $timeout:ident)?) => {
        fn $method<'call>(
            &'call self,
            log: EvalLogging<'call, '_>,
            call: $call<'call>,
        ) -> $pending<'call> {
            Box::pin(async move {
                let connection = EvalConnection {
                    config: call.config,
                    client: self.client,
                    url: &call.url,
                    headers: &call.headers,
                };
                let mut logging = log;
                $handler(&connection, &mut logging, $(Some(call.$timeout),)? &call.$payload)
                    .await
                    .map_err(EvalRunCallError::Transport)
            })
        }
    };
}

impl RunTransport for HttpRunTransport<'_> {
    run_handler!(
        create_run,
        CreateRunCall,
        PendingCreateRun,
        create_run_handler,
        payload body,
        timeout timeout
    );

    // LiteLLM's list GET sends no timeout, so the handler leaves the client's own deadline in place.
    run_handler!(
        list_runs,
        ListRunsCall,
        PendingListRuns,
        list_runs_handler,
        payload query
    );
}
