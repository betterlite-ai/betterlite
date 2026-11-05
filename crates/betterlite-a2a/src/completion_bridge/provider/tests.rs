#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use betterlite_core::agents::json::AgentJson;
use futures_core::Stream;
use rstest::rstest;
use serde_json::error::Category;

use super::super::events::{StreamEvent, StreamResult};
use super::super::streaming::{EventContent, stream_event};
use super::super::tests::Source;
use super::super::wire::{MessageSendParams, SendMessageResponse};
use super::super::{A2AStreamingContext, extract_text, response_to_a2a};
use super::BaseA2AProviderConfig;

struct Provider;
struct EventStream(Option<StreamEvent>);

impl Stream for EventStream {
    type Item = Result<StreamEvent, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.0.take().map(Ok))
    }
}

impl BaseA2AProviderConfig for Provider {
    type Error = Infallible;
    type EventStream = EventStream;

    fn handle_non_streaming(
        &self,
        request_id: &str,
        params: &MessageSendParams,
        _api_base: Option<&str>,
        _kwargs: &BTreeMap<String, AgentJson>,
    ) -> impl Future<Output = Result<SendMessageResponse, Self::Error>> + Send {
        let mut response = response_to_a2a(None, Some(request_id), &mut Source::default());
        response.result.parts.first_mut().unwrap().text = extract_text(&params.message.parts);
        ready(Ok(response))
    }

    fn handle_streaming(
        &self,
        request_id: &str,
        params: &MessageSendParams,
        api_base: Option<&str>,
        kwargs: &BTreeMap<String, AgentJson>,
    ) -> impl Future<Output = Result<Self::EventStream, Self::Error>> + Send {
        assert_eq!(params.message.role.0, "user");
        assert!(!params.extra.contains_key("message"));
        assert!(api_base.is_none());
        assert!(kwargs.is_empty());
        let mut source = Source::default();
        let context =
            A2AStreamingContext::new(request_id.to_owned(), params.message.clone(), &mut source);
        ready(Ok(EventStream(Some(stream_event(
            &context,
            EventContent::Artifact(&extract_text(&params.message.parts)),
            &mut source,
        )))))
    }
}

#[test]
fn provider_contract_is_awaitable() {
    let provider = Provider;
    let params =
        serde_json::from_str(r#"{"message":{"parts":[{"kind":"text","text":"hello"}]}}"#).unwrap();
    let kwargs = serde_json::from_str(r#"{"trace":"id"}"#).unwrap();
    let mut context = Context::from_waker(Waker::noop());
    let mut call = Box::pin(provider.handle_non_streaming(
        "request",
        &params,
        Some("https://agent.test"),
        &kwargs,
    ));
    let Poll::Ready(Ok(response)) = call.as_mut().poll(&mut context) else {
        panic!("response was not ready");
    };
    assert_eq!(response.id.as_deref(), Some("request"));
    assert_eq!(response.result.parts[0].text, "hello");
    let empty = BTreeMap::new();
    let mut streaming = Box::pin(provider.handle_streaming("stream", &params, None, &empty));
    let Poll::Ready(Ok(events)) = streaming.as_mut().poll(&mut context) else {
        panic!("stream was not ready");
    };
    let mut pinned = Box::pin(events);
    let Poll::Ready(Some(Ok(event))) = pinned.as_mut().poll_next(&mut context) else {
        panic!("missing artifact");
    };
    assert_eq!(event.id, "stream");
    let StreamResult::Artifact(result) = event.result else {
        panic!("expected artifact");
    };
    assert_eq!(result.artifact.parts[0].text, "hello");
    assert!(matches!(
        pinned.as_mut().poll_next(&mut context),
        Poll::Ready(None)
    ));
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"message":null}"#)]
#[case::wrong_type(r#"{"message":"hello"}"#)]
#[case::invalid_role(r#"{"message":{"role":1}}"#)]
fn send_params_require_message(#[case] input: &str) {
    let error = serde_json::from_str::<MessageSendParams>(input).unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[test]
fn send_params_preserve_fields() {
    let input = r#"{"message":{"role":"user","parts":[]},"metadata":{"tenant":"test"},"configuration":{"blocking":false},"extension":[1,2]}"#;
    let params: MessageSendParams = serde_json::from_str(input).unwrap();
    assert_eq!(params.message.role.0, "user");
    assert!(!params.extra.contains_key("message"));
    assert!(!params.extra.contains_key("metadata"));
    assert_eq!(
        serde_json::to_value(params).unwrap(),
        serde_json::from_str::<AgentJson>(input).unwrap()
    );
}
