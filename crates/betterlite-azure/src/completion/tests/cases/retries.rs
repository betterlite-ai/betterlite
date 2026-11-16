use betterlite_models::decisions::Json;

use crate::completion::arguments::SentCall;
use crate::completion::error::MAX_RETRIES_TYPE;
use crate::completion::handler::streaming::streaming;
use crate::completion::request::Retries;
use crate::completion::tests::fixture::{Fixed, Recorder, ScriptedSource, object, run};
use crate::endpoints::projection::ClientKind;

use super::support::{
    MODEL, Parts, async_buffered, buffered, field, number, runtime, sync_buffered, text_messages,
};

fn accepted_counts() {
    for value in ["true", "-1", "18446744073709551615"] {
        let params = format!(r#"{{"max_retries":{value}}}"#);
        assert!(sync_buffered(&params).execute().reply.is_some());
    }
    let fractional = sync_buffered(r#"{"max_retries":1.5}"#).execute();
    assert_eq!(fractional.failure().status, Some(422));
}

pub(super) fn max_retries_default_two() {
    let defaulted = sync_buffered("{}").execute();
    assert_eq!(defaulted.create("max_retries"), Json::Null);
    let rejected = sync_buffered(r#"{"max_retries":null}"#).execute();
    assert_eq!(rejected.failure().status, Some(422));
    assert_eq!(rejected.failure().message, MAX_RETRIES_TYPE);
    assert_eq!(rejected.hooks.pre.len(), 1);
    let unchecked = async_buffered(r#"{"max_retries":null}"#).execute();
    assert!(unchecked.reply.is_some());
    accepted_counts();
    let mut gateway = sync_buffered("{}");
    gateway.parts.api_base = Some("https://gateway.ai.cloudflare.com/gw".to_owned());
    let gateway_run = gateway.execute();
    let record = &gateway_run.source.gateways()[0];
    assert_eq!(number(&field(record, "max_retries")), 2);
}

fn direct_stream_guard() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let fixed = Fixed::default();
    let parts = Parts::default();
    let messages = text_messages();
    let mut params = object("{}");
    let call = parts.call(Some(&messages), &mut params, false);
    let mut sent = SentCall {
        model: MODEL.to_owned(),
        data: object(r#"{"max_retries":"3","model":"x","prompt":"p"}"#),
        prompt: String::new(),
        max_retries: Retries(Json::Null),
        litellm_params: &parts.litellm_params,
        supplied: None,
    };
    let mut hooks = Recorder::default();
    let guarded = run(streaming(
        &source,
        &mut sent,
        &call,
        &runtime(&fixed),
        &mut hooks,
    ));
    let Err(error) = guarded else {
        panic!("the retry count guard rejects the string");
    };
    assert_eq!(error.status, Some(422));
    assert!(source.gets().is_empty());
    assert!(!sent.data.contains_key("max_retries"));
}

pub(super) fn invalid_retries_422() {
    let run_result = sync_buffered(r#"{"max_retries":"3"}"#).execute();
    assert_eq!(run_result.failure().status, Some(422));
    assert_eq!(run_result.failure().message, MAX_RETRIES_TYPE);
    assert_eq!(run_result.hooks.pre.len(), 1);
    assert!(run_result.source.gets().is_empty());
    direct_stream_guard();
}
