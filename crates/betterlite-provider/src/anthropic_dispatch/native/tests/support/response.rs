use super::{Observed, TIMEOUT};
use crate::anthropic_dispatch::DispatchError;
use crate::anthropic_dispatch::native::{Failure, Reply, legacy::Reply as LegacyReply};
use crate::anthropic_dispatch::operations::Outcome;
use betterlite_anthropic::anthropic_http_stream::error::StreamFailure;
use betterlite_anthropic::chat_call::ChatOutcome;
use futures_util::future::LocalBoxFuture;
use std::convert::Infallible;
use tokio::time::timeout;

type NativeOutcome =
    Result<Outcome<Reply<StreamFailure>>, DispatchError<Failure<StreamFailure, Infallible>>>;

pub(super) fn observe(outcome: NativeOutcome) -> LocalBoxFuture<'static, Observed> {
    Box::pin(async move {
        let mut observed = Observed::default();
        match outcome {
            Ok(Outcome::Completion(Reply::Messages(ChatOutcome::Reply(reply)))) => {
                observed.posts = 1;
                observed
                    .content
                    .clone_from(&reply.choices[0].message.content);
                observed.tool_name.clone_from(
                    &reply.choices[0].message.tool_calls.as_ref().unwrap()[0]
                        .function
                        .name,
                );
            }
            Ok(Outcome::Completion(Reply::Legacy(reply))) => return legacy(reply).await,
            Err(DispatchError::Completion(Failure::Chat(error))) => {
                observed.status = error.status_code()
            }
            Err(DispatchError::Completion(Failure::Prepare(error))) => {
                observed.status = error.status_code();
                observed.preparation_error = Some(error);
            }
            Err(DispatchError::Completion(Failure::Config)) => {}
            _ => panic!("unexpected native outcome"),
        }
        observed
    })
}

async fn legacy(reply: LegacyReply) -> Observed {
    let content = match reply {
        LegacyReply::Buffered(envelope) => envelope.choices[0].message.content.clone(),
        LegacyReply::Stream { response, .. } => {
            let buffered = timeout(TIMEOUT, (*response).buffered(0x0001_0000))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(buffered.headers["content-type"], "text/event-stream");
            Some(String::from_utf8(buffered.body).unwrap())
        }
    };
    Observed {
        posts: 1,
        content,
        body: super::wire::Body::default(),
        key: String::new(),
        status: None,
        logged: String::new(),
        tool_name: None,
        preparation_error: None,
    }
}
