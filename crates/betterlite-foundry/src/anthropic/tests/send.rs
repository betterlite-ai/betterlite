#![cfg(test)]

use std::future::Future;
use std::thread;

use betterlite_anthropic::chat_call::ChatClients;
use betterlite_http::client_kind::SuppliedClient;
use tokio::runtime::Builder;

use crate::anthropic::handler::completion;

use super::harness::{
    Outcome, Setup, deps_of, fixture_config, outcome_of, request_of, signing_material,
};
use super::support::{
    FixtureTransport, RecordingLogger, Silent, logging_env, logging_state, redactor,
};

// blocks on one call's future on the second thread and gives back what that thread received
fn on_second_thread<Output, Call>(future: Call) -> Output
where
    Output: Send,
    Call: Future<Output = Output> + Send,
{
    thread::scope(|scope| {
        scope
            .spawn(|| {
                Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(future)
            })
            .join()
            .unwrap()
    })
}

// the Foundry completion completes on a thread that is not the one that built it
#[test]
fn the_foundry_completion_completes_on_a_second_thread() {
    let setup = Setup::chat();
    let redactor = redactor();
    let sink = Silent;
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor, &sink);
    let config = fixture_config(&setup);
    let supplied = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let pooled = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let logger = RecordingLogger::default();
    let (credentials, keys) = signing_material(&setup);
    let request = request_of(&setup, &credentials, &keys);
    let clients = ChatClients {
        supplied: setup.supplied.then_some(SuppliedClient {
            kind: setup.supplied_kind,
            transport: &supplied,
        }),
        pooled: &pooled,
    };
    let deps = deps_of(&config, clients, &env, &mut state, &logger);
    let result = on_second_thread(completion(request, deps));
    let outcome = outcome_of(result).expect("the fixture replies");
    let Outcome::Reply(envelope) = outcome else {
        panic!("a buffered completion returns the reply");
    };
    let content = envelope
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref());
    assert_eq!(content, Some("hello"));
}
