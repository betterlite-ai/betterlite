use super::MEMBER_KIND;
use super::naming::DEFAULT_SCIM_MEMBER_VALUE;
use crate::scim_records::group::{Member, MemberKind};
use betterlite_db::team_rows::team::LiteLLMTeamTable;

#[must_use]
pub fn user_member(value: &str, display: &str) -> Member {
    Member {
        value: value.to_owned(),
        display: Some(display.to_owned()),
        kind: MemberKind(Some(MEMBER_KIND.to_owned())),
    }
}

// a roster entry can outlive its user row, so the value falls back to a placeholder instead of failing
#[must_use]
pub fn project_member(user_id: Option<&str>, user_email: Option<&str>) -> Member {
    let non_empty = |text: &&str| !text.is_empty();
    let id = user_id.filter(non_empty);
    user_member(
        id.unwrap_or(DEFAULT_SCIM_MEMBER_VALUE),
        user_email
            .filter(non_empty)
            .or(id)
            .unwrap_or(DEFAULT_SCIM_MEMBER_VALUE),
    )
}

// an empty user_id still counts as a member, as only None is skipped
#[must_use]
pub fn team_member_user_ids(team: &LiteLLMTeamTable) -> Vec<String> {
    team.fields
        .base
        .members_with_roles
        .iter()
        .filter_map(|member| member.base.user_id.clone())
        .collect()
}
