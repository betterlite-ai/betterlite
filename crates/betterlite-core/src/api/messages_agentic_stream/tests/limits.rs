#![cfg(test)]

use rstest::rstest;

use super::{
    EffectLog, HookReply, HookSeen, ScriptedInner, ScriptedInterceptor, agentic_input,
    immediate_sleep, resolved_interceptor, text_stream,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::messages_agentic_stream::MessagesAgenticError;
use crate::api::messages_agentic_stream::stream::MessagesAgenticStream;
use crate::api::messages_sse_events::stream::MessagesProviderStream as _;

type Read = Result<Option<Vec<u8>>, MessagesAgenticError>;

#[derive(Clone, Copy)]
enum Cap {
    Fits,
    Bytes,
    Chunks,
}

async fn read_times(
    stream: &mut MessagesAgenticStream<ScriptedInner, ScriptedInterceptor>,
    count: usize,
) -> Vec<Read> {
    let mut reads = Vec::new();
    while reads.len() < count {
        reads.push(stream.next_chunk().await);
    }
    reads
}

// the collected provider chunks stop at the byte and the item boundary, so neither long nor empty chunks grow the buffer
#[rstest]
#[case::text_at_limit(text_stream(), Cap::Fits)]
#[case::text_over_bytes(text_stream(), Cap::Bytes)]
#[case::empty_chunks_at_limit(vec![Vec::new(); 8], Cap::Fits)]
#[case::empty_chunks_over_count(vec![Vec::new(); 8], Cap::Chunks)]
#[tokio::test]
async fn collected_bytes_respect_limits(#[case] frames: Vec<Vec<u8>>, #[case] cap: Cap) {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let total = frames.iter().map(Vec::len).sum::<usize>();
    let count = frames.len();
    let (max_json_bytes, max_items, expected) = match cap {
        Cap::Fits => (total, count, None),
        Cap::Bytes => (
            total.saturating_sub(1),
            count,
            Some(ApiLimitError::JsonTooLarge {
                observed: total,
                limit: total.saturating_sub(1),
            }),
        ),
        Cap::Chunks => (
            total,
            count.saturating_sub(1),
            Some(ApiLimitError::TooManyItems {
                observed: count,
                limit: count.saturating_sub(1),
            }),
        ),
    };
    let limits = ApiLimits {
        max_json_bytes,
        max_event_bytes: ApiLimits::default().max_event_bytes,
        max_items,
    };
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames, &effects),
        resolved_interceptor(HookReply::Empty, &effects, &seen),
        agentic_input(None),
        immediate_sleep(),
        limits,
    );
    let mut reads = read_times(&mut stream, count).await;
    let last = reads.pop().expect("one read per frame");
    assert!(reads.iter().all(Result::is_ok));
    let failure = match last {
        Ok(_chunk) => None,
        Err(MessagesAgenticError::Limit(limit)) => Some(limit),
        Err(other) => panic!("the read fails with {other}"),
    };
    assert_eq!(failure, expected);
}
