use serde::{Deserialize, Serialize};

use super::caller::MemoryCaller;

// _visibility_filter's answer for a caller with no identity: no row can match it
const NO_MATCH: &str = "__no_match__";

// the Prisma where fragments the memory helpers build: a field equality, a key match, or OR/AND groups
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MemoryWhere {
    UserId {
        user_id: String,
    },
    TeamId {
        team_id: String,
    },
    MemoryId {
        memory_id: String,
    },
    Key {
        key: KeyMatch,
    },
    AnyOf {
        #[serde(rename = "OR")]
        any_of: Vec<Self>,
    },
    AllOf {
        #[serde(rename = "AND")]
        all_of: Vec<Self>,
    },
}

// _StartsWith, and the exact-or-prefix split of _MemoryKeyWhere and _MemorySearchWhere
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyMatch {
    Exact(String),
    StartsWith {
        #[serde(rename = "startsWith")]
        starts_with: String,
    },
}

// memory_endpoints.py _visibility_filter: the admin view reads every row, everyone else their user's or team's
#[must_use]
pub fn visibility_filter(caller: &MemoryCaller) -> Option<MemoryWhere> {
    if caller.admin_view {
        return None;
    }
    let mut any_of = Vec::new();
    if let Some(user_id) = caller.user_id.as_deref() {
        any_of.push(MemoryWhere::UserId {
            user_id: user_id.to_owned(),
        });
    }
    if let Some(team_id) = caller.team_id.as_deref() {
        any_of.push(MemoryWhere::TeamId {
            team_id: team_id.to_owned(),
        });
    }
    if any_of.is_empty() {
        return Some(MemoryWhere::MemoryId {
            memory_id: NO_MATCH.to_owned(),
        });
    }
    Some(MemoryWhere::AnyOf { any_of })
}

// memory_endpoints.py _key_filter: search beats key_prefix beats key, and an empty string is supplied
#[must_use]
pub fn key_filter(
    search: Option<&str>,
    key_prefix: Option<&str>,
    key: Option<&str>,
) -> Option<MemoryWhere> {
    if let Some(text) = search {
        return Some(MemoryWhere::AnyOf {
            any_of: vec![
                MemoryWhere::Key {
                    key: KeyMatch::StartsWith {
                        starts_with: text.to_owned(),
                    },
                },
                MemoryWhere::MemoryId {
                    memory_id: text.to_owned(),
                },
            ],
        });
    }
    if let Some(prefix) = key_prefix {
        return Some(MemoryWhere::Key {
            key: KeyMatch::StartsWith {
                starts_with: prefix.to_owned(),
            },
        });
    }
    key.map(|exact| MemoryWhere::Key {
        key: KeyMatch::Exact(exact.to_owned()),
    })
}
