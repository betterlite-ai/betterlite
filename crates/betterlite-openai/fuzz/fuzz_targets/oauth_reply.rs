#![no_main]

use betterlite_openai::chatgpt::login::codec::{
    AuthorizationReply, DeviceReply, TokenReply, decode_chatgpt_oauth,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _device = decode_chatgpt_oauth::<DeviceReply>(bytes).map(DeviceReply::into_code);
    let _authorization =
        decode_chatgpt_oauth::<AuthorizationReply>(bytes).map(AuthorizationReply::complete);
    let _tokens = decode_chatgpt_oauth::<TokenReply>(bytes).map(TokenReply::exchanged);
});
