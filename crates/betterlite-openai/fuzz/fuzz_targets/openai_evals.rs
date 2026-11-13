#![no_main]
use betterlite_openai::evals::decode::decode_eval_reply;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_eval_reply(bytes);
});
