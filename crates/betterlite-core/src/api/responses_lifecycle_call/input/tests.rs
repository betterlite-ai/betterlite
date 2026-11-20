use secrecy::zeroize::Zeroize as _;

use super::LifecycleApiKey;

// the SecretBox wipes the key through this impl, so a no-op would leave the text readable
#[test]
fn lifecycle_api_key_zeroizes() {
    let mut key = LifecycleApiKey(String::from("sk-request"));
    key.zeroize();
    assert!(key.0.bytes().all(|byte| byte == 0));
}
