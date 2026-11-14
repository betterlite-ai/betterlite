use std::collections::BTreeSet;

use super::classified::{ClassifiedGroupMember, SkipReason, roster_entries_named_by};
use super::error::ScimMemberIdentityError;
use super::lookups::MemberResolver;
use super::member_value::{member_value, normalized_member_type};
use crate::scim_records::group::Member;

fn skipped_member(value: &str, reason: SkipReason) -> ClassifiedGroupMember {
    ClassifiedGroupMember::Skipped {
        value: value.to_owned(),
        reason,
    }
}

// a value that is one account's exact user_id is still ambiguous when another account uses it as an identity
fn keyed_resolution(value: &str, named: &[String]) -> ClassifiedGroupMember {
    if named.iter().any(|other| other != value) {
        return ClassifiedGroupMember::Ambiguous {
            value: value.to_owned(),
        };
    }
    ClassifiedGroupMember::Resolved {
        user_id: value.to_owned(),
    }
}

fn named_resolution(value: &str, named: &[String]) -> ClassifiedGroupMember {
    match named {
        [] => ClassifiedGroupMember::Unknown {
            value: value.to_owned(),
        },
        [user_id] => ClassifiedGroupMember::Resolved {
            user_id: user_id.clone(),
        },
        _ => ClassifiedGroupMember::Ambiguous {
            value: value.to_owned(),
        },
    }
}

impl MemberResolver<'_> {
    pub async fn classify_group_member(
        &self,
        member: &Member,
    ) -> Result<ClassifiedGroupMember, ScimMemberIdentityError> {
        let value = member_value(member)?;
        let member_type = normalized_member_type(member);
        if member_type.as_deref() == Some("group") {
            return Ok(skipped_member(value, SkipReason::NestedGroup));
        }
        let named = self.accounts_named_by_member_value(value).await?;
        if named.iter().any(|found| found == value) {
            return Ok(keyed_resolution(value, &named));
        }
        if member_type.as_deref().is_some_and(|kind| kind != "user") {
            return Ok(skipped_member(value, SkipReason::NonUserType));
        }
        if member_type.is_none() && self.is_scim_managed_team(value).await? {
            return Ok(skipped_member(value, SkipReason::ExistingTeam));
        }
        Ok(named_resolution(value, &named))
    }

    // every account sharing the value leaves with the one member of this roster it names
    pub async fn member_ids_to_drop(
        &self,
        members: &[Member],
        roster: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, ScimMemberIdentityError> {
        let mut written: BTreeSet<&str> = BTreeSet::new();
        for member in members {
            written.insert(member_value(member)?);
        }
        let mut matched: Vec<(String, Vec<String>)> = Vec::new();
        for value in written {
            let resolved = self.users_named_by_member_value(value, None).await?;
            matched.push((
                value.to_owned(),
                roster_entries_named_by(value, roster, &resolved),
            ));
        }
        if let Some((value, _)) = matched.iter().find(|(_, entries)| entries.len() > 1) {
            return Err(ScimMemberIdentityError::AmbiguousRemoval {
                value: value.clone(),
            });
        }
        Ok(matched
            .into_iter()
            .flat_map(|(_, entries)| entries)
            .collect())
    }
}
