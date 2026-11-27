use caseless::default_case_fold_str;
use sha2::{Digest as _, Sha256};

use super::source::source_group;

pub const CACHE_KEY_PREFIX: &str = "login_fail";
pub const UNKNOWN_SOURCE: &str = "unknown";

// one cluster slot: every key carries the same {group} hash tag
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginKeys {
    pub pair_counter: String,
    pub pair_block: String,
    pub source_counter: String,
    pub source_block: String,
}

impl LoginKeys {
    #[must_use]
    pub fn of(client_ip: &str, username: &str) -> Self {
        let group = source_group(client_ip);
        let user = hex::encode(Sha256::digest(default_case_fold_str(username).as_bytes()));
        Self {
            pair_counter: format!("{CACHE_KEY_PREFIX}:{{{group}}}:user:{user}"),
            pair_block: format!("{CACHE_KEY_PREFIX}:{{{group}}}:block:user:{user}"),
            source_counter: format!("{CACHE_KEY_PREFIX}:{{{group}}}:source"),
            source_block: format!("{CACHE_KEY_PREFIX}:{{{group}}}:block:source"),
        }
    }

    // KEYS order for both Lua scripts: pair counter, pair block, source counter, source block
    #[must_use]
    pub fn all(&self) -> [String; 4] {
        [
            self.pair_counter.clone(),
            self.pair_block.clone(),
            self.source_counter.clone(),
            self.source_block.clone(),
        ]
    }
}
