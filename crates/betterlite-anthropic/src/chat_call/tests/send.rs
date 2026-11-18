#![cfg(test)]

use std::thread;

use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::chat_call::transport::SuppliedChatClient;
use crate::chat_call::{ChatClients, chat_call};

use super::fixture::{
    CALLER_KEY, FixtureTransport, RecordingLogger, Silent, logging_env, logging_state, redactor,
};
use super::harness::{Outcome, Setup, call_of, deps_of, fixture_config, outcome_of, settings_of};

// the buffered call completes on a thread that is not the one that built it
#[test]
fn chat_call_completes_on_a_second_thread() {
    let setup = Setup::chat();
    let redactor = redactor();
    let sink = Silent;
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor, &sink);
    let config = fixture_config(&setup);
    let supplied = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let pooled = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let logger = RecordingLogger::default();
    let settings = settings_of(&setup);
    let caller_key = SecretString::from(CALLER_KEY.to_owned());
    let call = call_of(&setup, Some(&caller_key), &settings);
    let clients = ChatClients {
        supplied: setup.supplied.then_some(SuppliedChatClient {
            kind: setup.supplied_kind,
            transport: &supplied,
        }),
        pooled: &pooled,
    };
    let deps = deps_of(&config, clients, &env, &mut state, &logger);
    let future = chat_call(call, deps);
    let result = thread::scope(|scope| {
        scope
            .spawn(|| {
                Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(future)
            })
            .join()
            .unwrap()
    });
    let outcome = outcome_of(result).expect("the fixture replies");
    let Outcome::Reply(envelope) = outcome else {
        panic!("a buffered call returns the reply");
    };
    let content = envelope
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref());
    assert_eq!(content, Some("hello"));
}
