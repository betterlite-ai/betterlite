use std::collections::BTreeMap;

use super::identity::MemberIdentity;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::team_rows::member::Member;

#[must_use]
pub fn same_email(candidate_email: Option<&str>, request: &MemberIdentity) -> bool {
    match (request.user_email.as_deref(), candidate_email) {
        (Some(requested), Some(candidate)) => requested == candidate,
        _ => false,
    }
}

#[must_use]
pub fn addresses_member(member: &Member, request: &MemberIdentity) -> bool {
    let Some(requested_id) = &request.user_id else {
        return same_email(member.base.user_email.as_deref(), request);
    };
    member.base.user_id.as_deref() == Some(requested_id.as_str())
        || (member.base.user_id.is_none() && same_email(member.base.user_email.as_deref(), request))
}

#[must_use]
pub fn addresses_user(user: &LiteLLMUserTable, request: &MemberIdentity) -> bool {
    let matches_requested_id = request
        .user_id
        .as_ref()
        .is_some_and(|requested_id| user.user_id == *requested_id);
    let matches_by_email =
        request.user_id.is_none() && same_email(user.user_email.as_deref(), request);
    matches_requested_id || matches_by_email
}

#[must_use]
pub fn with_row_email(
    request: MemberIdentity,
    email_of: &BTreeMap<String, String>,
) -> MemberIdentity {
    let MemberIdentity {
        user_id,
        user_email,
    } = request;
    if user_email.is_some() {
        return MemberIdentity {
            user_id,
            user_email,
        };
    }
    match &user_id {
        None => MemberIdentity {
            user_id,
            user_email,
        },
        Some(id) => {
            let filled_email = email_of.get(id).cloned();
            MemberIdentity {
                user_id,
                user_email: filled_email,
            }
        }
    }
}
