#![no_main]

use betterlite_bedrock::realtime_events::decode::{
    decode_realtime_client_preface, decode_realtime_native_event,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    drop(decode_realtime_native_event(input));
    drop(decode_realtime_client_preface(input));
});
