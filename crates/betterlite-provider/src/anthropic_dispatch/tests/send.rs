#![cfg(test)]

use std::thread;

use tokio::runtime::Builder;

use super::super::fixtures::execute;
use super::single_chat_case;

// the dispatch entry completes on a thread that is not the one that built it
#[test]
fn the_dispatch_entry_completes_on_a_second_thread() {
    let case = single_chat_case().call.anthropic_provider;
    let future = execute(&case);
    let actual = thread::scope(|scope| {
        scope
            .spawn(|| {
                Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(future)
                    .unwrap()
            })
            .join()
            .unwrap()
    });
    assert_eq!(actual, case.expected);
}
