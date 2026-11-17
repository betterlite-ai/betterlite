#![no_main]

use betterlite_foundry::agent_runs::decode::{
    decode_agent_messages, decode_agent_run_status, decode_created_id,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _status = decode_agent_run_status(bytes);
    let _created = decode_created_id(bytes);
    let _messages = decode_agent_messages(bytes);
});
