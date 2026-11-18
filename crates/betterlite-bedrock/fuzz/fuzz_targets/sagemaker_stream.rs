#![no_main]
use std::collections::VecDeque;

use betterlite_bedrock::sagemaker_stream::events::EventState;
use betterlite_bedrock::sagemaker_stream::{ErrorPolicy, SagemakerEvents};
use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let size = usize::from(bytes.first().copied().unwrap_or(0)).max(1);
    let data = bytes.get(1..).unwrap_or_default();
    for policy in [ErrorPolicy::Propagate, ErrorPolicy::Skip] {
        for messages in [false, true] {
            let chunks = data
                .chunks(size)
                .map(|chunk| Ok(chunk.to_vec()))
                .collect::<Vec<_>>();
            let events = SagemakerEvents {
                state: EventState {
                    model: String::from("sagemaker-model"),
                    is_messages_api: messages,
                    policy,
                    frames: BytesMut::new(),
                    accumulated: String::new(),
                },
                chunks: chunks.into_iter(),
                pending: VecDeque::new(),
                done: false,
            };
            drop(events.collect::<Vec<_>>());
        }
    }
});
