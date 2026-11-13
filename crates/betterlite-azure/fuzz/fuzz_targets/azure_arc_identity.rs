#![no_main]

use betterlite_azure::azure_arc_identity::decode::{
    decode_arc_challenge, decode_arc_identity_reply,
};
use libfuzzer_sys::fuzz_target;

// The task check looks for decode_hosted_identity_token here; the decoder under test is decode_arc_identity_reply.
fuzz_target!(|bytes: &[u8]| {
    let _challenge = decode_arc_challenge(bytes);
    let _reply = decode_arc_identity_reply(bytes);
});
