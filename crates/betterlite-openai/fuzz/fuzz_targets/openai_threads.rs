#![no_main]

use betterlite_openai::threads::decode::decode_thread_message_reply;
use betterlite_openai::threads::records::ThreadMessage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _message = decode_thread_message_reply::<ThreadMessage>(bytes);
});
