use std::collections::BTreeMap;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::member_removal_identity::duplicates::duplicate_member_indexes;
use betterlite_db::member_removal_identity::filters::{
    AnyFilter, FilterClause, eq_filter, in_filter, team_users_filter,
};
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::member_removal_identity::matching::{
    addresses_member, addresses_user, same_email, with_row_email,
};
use betterlite_db::team_rows::member::Member;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "validate")]
    Validate {
        user_id: Option<String>,
        user_email: Option<String>,
    },
    #[serde(rename = "same_email")]
    SameEmail {
        candidate_email: Option<String>,
        user_id: Option<String>,
        user_email: Option<String>,
    },
    #[serde(rename = "addresses_member")]
    AddressesMember {
        member: Box<Member>,
        user_id: Option<String>,
        user_email: Option<String>,
    },
    #[serde(rename = "addresses_user")]
    AddressesUser {
        user: Box<LiteLLMUserTable>,
        user_id: Option<String>,
        user_email: Option<String>,
    },
    #[serde(rename = "with_row_email")]
    WithRowEmail {
        user_id: Option<String>,
        user_email: Option<String>,
        email_of: BTreeMap<String, String>,
    },
    #[serde(rename = "duplicate_indexes")]
    DuplicateIndexes { requests: Vec<MemberIdentity> },
    #[serde(rename = "eq_filter")]
    EqFilter { field: String, value: String },
    #[serde(rename = "in_filter")]
    InFilter { field: String, values: Vec<String> },
    #[serde(rename = "team_users_filter")]
    TeamUsersFilter {
        team_id: String,
        user_ids: Vec<String>,
    },
    #[serde(rename = "any_filter")]
    AnyFilter { clauses: Vec<FilterClause> },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Validate {
                user_id,
                user_email,
            } => run_validate(user_id.as_deref(), user_email.as_deref()),
            Input::SameEmail {
                candidate_email,
                user_id,
                user_email,
            } => run_same_email(
                candidate_email.as_deref(),
                user_id.as_deref(),
                user_email.as_deref(),
            ),
            Input::AddressesMember {
                member,
                user_id,
                user_email,
            } => run_addresses_member(member, user_id.as_deref(), user_email.as_deref()),
            Input::AddressesUser {
                user,
                user_id,
                user_email,
            } => run_addresses_user(user, user_id.as_deref(), user_email.as_deref()),
            Input::WithRowEmail {
                user_id,
                user_email,
                email_of,
            } => run_with_row_email(user_id.as_deref(), user_email.as_deref(), email_of),
            Input::DuplicateIndexes { requests } => run_duplicate_indexes(requests),
            Input::EqFilter { field, value } => to_json(&eq_filter(field, value.clone())),
            Input::InFilter { field, values } => to_json(&in_filter(field, values.clone())),
            Input::TeamUsersFilter { team_id, user_ids } => {
                to_json(&team_users_filter(team_id.clone(), user_ids.clone()))
            }
            Input::AnyFilter { clauses } => to_json(&AnyFilter {
                or: clauses.clone(),
            }),
        }
    }
}

fn run_validate(user_id: Option<&str>, user_email: Option<&str>) -> crate::helpers::HelperResult {
    let identity = build_identity(user_id, user_email)?;
    to_json(&identity)
}

fn run_same_email(
    candidate_email: Option<&str>,
    user_id: Option<&str>,
    user_email: Option<&str>,
) -> crate::helpers::HelperResult {
    let request = build_identity(user_id, user_email)?;
    to_json(&same_email(candidate_email, &request))
}

fn run_addresses_member(
    member: &Member,
    user_id: Option<&str>,
    user_email: Option<&str>,
) -> crate::helpers::HelperResult {
    let request = build_identity(user_id, user_email)?;
    to_json(&addresses_member(member, &request))
}

fn run_addresses_user(
    user: &LiteLLMUserTable,
    user_id: Option<&str>,
    user_email: Option<&str>,
) -> crate::helpers::HelperResult {
    let request = build_identity(user_id, user_email)?;
    to_json(&addresses_user(user, &request))
}

fn run_with_row_email(
    user_id: Option<&str>,
    user_email: Option<&str>,
    email_of: &BTreeMap<String, String>,
) -> crate::helpers::HelperResult {
    let request = build_identity(user_id, user_email)?;
    to_json(&with_row_email(request, email_of))
}

fn run_duplicate_indexes(requests: &[MemberIdentity]) -> crate::helpers::HelperResult {
    let indexes = duplicate_member_indexes(requests);
    to_json(&indexes.into_iter().collect::<Vec<usize>>())
}

fn build_identity(
    user_id: Option<&str>,
    user_email: Option<&str>,
) -> Result<MemberIdentity, Box<crate::helpers::HelperError>> {
    let outcome = MemberIdentity::new(user_id.map(str::to_owned), user_email.map(str::to_owned));
    outcome.map_err(|error| helper_error("ValidationError", &error.to_string()))
}

fn to_json<T>(value: &T) -> crate::helpers::HelperResult
where
    T: serde::Serialize,
{
    let encoded = serde_json::to_value(value);
    encoded.map_err(|error| helper_error("InvalidOutput", &error.to_string()))
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
