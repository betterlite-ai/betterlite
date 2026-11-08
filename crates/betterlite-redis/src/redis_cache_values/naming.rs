use super::RedisCacheValues;
use crate::cache_primitives::namespace::check_and_fix_namespace;

pub(crate) fn apply_namespace(values: &RedisCacheValues, key: &str) -> String {
    check_and_fix_namespace(values.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned())
}
