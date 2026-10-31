#![no_main]

use std::collections::BTreeMap;

use betterlite_redis::redis_connection_settings::model::settings::{ClientKind, RedisOverrides};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(overrides) = serde_json::from_slice::<RedisOverrides>(data) else {
        return;
    };
    let env: BTreeMap<String, String> = BTreeMap::new();
    drop(resolve_redis_kwargs(overrides, ClientKind::Sync, &env));
});
