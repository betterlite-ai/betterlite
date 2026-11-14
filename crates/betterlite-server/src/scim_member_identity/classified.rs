use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    NestedGroup,
    NonUserType,
    ExistingTeam,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedGroupMember {
    pub value: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClassifiedGroupMember {
    Resolved { user_id: String },
    Skipped { value: String, reason: SkipReason },
    Unknown { value: String },
    Ambiguous { value: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionedMembers {
    pub resolved_ids: Vec<String>,
    pub skipped: Vec<SkippedGroupMember>,
    pub unknown_ids: Vec<String>,
    pub ambiguous_values: Vec<String>,
}

// the four buckets keep the request's order and every duplicate, as LiteLLM's chain.from_iterable does
#[must_use]
pub fn partition_classified_members(classified: &[ClassifiedGroupMember]) -> PartitionedMembers {
    let mut partition = PartitionedMembers::default();
    for entry in classified {
        match entry {
            ClassifiedGroupMember::Resolved { user_id } => {
                partition.resolved_ids.push(user_id.clone());
            }
            ClassifiedGroupMember::Skipped { value, reason } => {
                partition.skipped.push(SkippedGroupMember {
                    value: value.clone(),
                    reason: *reason,
                });
            }
            ClassifiedGroupMember::Unknown { value } => partition.unknown_ids.push(value.clone()),
            ClassifiedGroupMember::Ambiguous { value } => {
                partition.ambiguous_values.push(value.clone());
            }
        }
    }
    partition
}

// an id the request repeats is one member, and only a resolved or newly created account is admitted
#[must_use]
pub fn admitted_member_ids(
    classified: &[ClassifiedGroupMember],
    created_ids: &BTreeSet<String>,
) -> Vec<String> {
    let mut admitted: Vec<String> = Vec::new();
    for entry in classified {
        let member_id = match entry {
            ClassifiedGroupMember::Resolved { user_id } => Some(user_id),
            ClassifiedGroupMember::Unknown { value } if created_ids.contains(value) => Some(value),
            ClassifiedGroupMember::Skipped { .. }
            | ClassifiedGroupMember::Ambiguous { .. }
            | ClassifiedGroupMember::Unknown { .. } => None,
        };
        if let Some(member_id) = member_id
            && !admitted.contains(member_id)
        {
            admitted.push(member_id.clone());
        }
    }
    admitted
}

// the id as written counts when the roster holds it verbatim, and so does each account it resolves to
#[must_use]
pub fn roster_entries_named_by(
    value: &str,
    roster: &BTreeSet<String>,
    resolved: &[String],
) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    if roster.contains(value) {
        entries.push(value.to_owned());
    }
    for user_id in resolved {
        if roster.contains(user_id) && !entries.contains(user_id) {
            entries.push(user_id.clone());
        }
    }
    entries
}
