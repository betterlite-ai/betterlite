#![no_main]

use betterlite_cli::commands::debug::wire::{RequestResponsePayload, SessionLogsPage};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _page = serde_json::from_slice::<SessionLogsPage>(bytes);
    let _payload = serde_json::from_slice::<Option<RequestResponsePayload>>(bytes);
});
