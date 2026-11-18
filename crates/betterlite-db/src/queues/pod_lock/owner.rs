use std::fmt;

// PodLockManager._COMPARE_AND_DELETE_LOCK_SCRIPT: delete only when the stored JSON owner equals ours.
pub const COMPARE_AND_DELETE_LOCK_SCRIPT: &str = r#"if redis.call("get", KEYS[1]) == ARGV[1] then
    return redis.call("del", KEYS[1])
else
    return 0
end
"#;

// PodLockManager.get_redis_lock_key: the lease key for one cron job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PodLockKey<'a>(pub &'a str);

impl fmt::Display for PodLockKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "cronjob_lock:{}", self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PodOwnerError {
    #[error("the cached pod owner is not valid UTF-8")]
    Utf8(#[from] std::str::Utf8Error),
}

// A cache returns bytes; a legacy cache stored the raw owner while B126 stores the JSON string, which the Redis adapter hands back unquoted.
pub fn decode_pod_owner(bytes: &[u8]) -> Result<String, PodOwnerError> {
    let owner = std::str::from_utf8(bytes)?;
    Ok(owner.to_owned())
}
