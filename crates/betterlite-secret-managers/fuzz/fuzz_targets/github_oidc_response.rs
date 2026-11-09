#![no_main]

use betterlite_secret_managers::secret_lookup::github_response::decode_github_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_github_token(bytes);
});
