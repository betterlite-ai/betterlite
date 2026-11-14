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
            // the keyed row still loses to an account that shares the value as an identity
            if named.iter().any(|other| other != value) {
                return Ok(ClassifiedGroupMember::Ambiguous {
                    value: value.to_owned(),
                });
            }
            return Ok(ClassifiedGroupMember::Resolved {
                user_id: value.to_owned(),
            });
        }
        if member_type.as_deref().is_some_and(|kind| kind != "user") {
            return Ok(skipped_member(value, SkipReason::NonUserType));
        }
        if member_type.is_none() && self.is_scim_managed_team(value).await? {
            return Ok(skipped_member(value, SkipReason::ExistingTeam));
        }
        match named.as_slice() {
            [] => Ok(ClassifiedGroupMember::Unknown {
                value: value.to_owned(),
            }),
            [user_id] => Ok(ClassifiedGroupMember::Resolved {
                user_id: user_id.clone(),
            }),
            _ => Ok(ClassifiedGroupMember::Ambiguous {
                value: value.to_owned(),
            }),
        }
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
