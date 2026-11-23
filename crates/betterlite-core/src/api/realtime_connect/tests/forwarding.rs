use std::sync::Arc;

use indexmap::IndexMap;

use crate::api::realtime_backend_transport::dial::RealtimeWebSocketDialer;

use super::super::RealtimeConnection;
use super::support::{
    Recorder, ScriptedTokens, TestConfig, UnusedDialer, call_with_client, params, run, secret,
    settings,
};

async fn connection_with_client(
    model: &str,
    dispatch: &Recorder,
    dialer: &Arc<dyn RealtimeWebSocketDialer>,
) -> RealtimeConnection<TestConfig> {
    let connect = params();
    let litellm_params = IndexMap::new();
    let request = call_with_client(model, &connect, dialer, &litellm_params);
    let (_, result) = run(dispatch, &settings(), &request).await;
    result.unwrap()
}

pub(super) async fn connector_branches_drop_the_client() {
    let dispatch = Recorder::default();
    let dialer: Arc<dyn RealtimeWebSocketDialer> = Arc::new(UnusedDialer);
    let connection = connection_with_client("openai/gpt-realtime", &dispatch, &dialer).await;
    assert!(connection.args.client.is_none());
}

pub(super) async fn vertex_forwards_the_client() {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(tokens);
    let dialer: Arc<dyn RealtimeWebSocketDialer> = Arc::new(UnusedDialer);
    let connection = connection_with_client("vertex_ai/gemini-live", &dispatch, &dialer).await;
    let forwarded = connection.args.client.as_ref().unwrap();
    assert!(Arc::ptr_eq(forwarded, &dialer));
}
