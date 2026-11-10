use betterlite_redis::cache_primitives::namespace::check_and_fix_namespace;
use betterlite_redis::redis_cache::ttl::remaining_ttl;
use serde_json::{Value, json};

use super::input::TtlReplyCall;

pub(crate) fn run(call: &TtlReplyCall) -> Value {
    let key = check_and_fix_namespace(call.namespace.as_deref(), Some(call.key.as_str()));
    json!({
        "key_sent": key,
        "ttl": remaining_ttl(call.reply),
    })
}
