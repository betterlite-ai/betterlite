use std::str::{self, Utf8Error};

use uuid::{Uuid, Variant, Version};

// PodLockManager._COMPARE_AND_DELETE_LOCK_SCRIPT: delete only when the stored JSON owner equals ours.
pub const COMPARE_AND_DELETE_LOCK_SCRIPT: &str = r#"if redis.call("get", KEYS[1]) == ARGV[1] then
    return redis.call("del", KEYS[1])
else
    return 0
end
"#;

// PodLockManager.get_redis_lock_key: the lease key for one cron job.
#[must_use]
pub fn lease_key(cronjob_id: &str) -> String {
    let mut key = String::from("cronjob_lock:");
    key.push_str(cronjob_id);
    key
}

#[derive(Debug, thiserror::Error)]
pub enum PodOwnerError {
    #[error("the cached pod owner is not valid UTF-8")]
    Utf8(#[from] Utf8Error),
    #[error("the pod owner must be an RFC 4122 UUID v4")]
    NotV4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodId(String);

impl TryFrom<Uuid> for PodId {
    type Error = PodOwnerError;

    fn try_from(owner: Uuid) -> Result<Self, Self::Error> {
        if owner.get_version() != Some(Version::Random) || owner.get_variant() != Variant::RFC4122 {
            return Err(PodOwnerError::NotV4);
        }
        Ok(Self(owner.to_string()))
    }
}

impl AsRef<str> for PodId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// A cache returns bytes; a legacy cache stored the raw owner while B126 stores the JSON string, which the Redis adapter hands back unquoted.
pub fn decode_pod_owner(bytes: &[u8]) -> Result<String, PodOwnerError> {
    match str::from_utf8(bytes) {
        Ok(owner) => Ok(owner.to_owned()),
        Err(source) => Err(PodOwnerError::Utf8(source)),
    }
}
