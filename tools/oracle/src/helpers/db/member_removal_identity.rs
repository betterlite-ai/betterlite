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
pub(crate) struct ValidateArgs {
    user_id: Option<String>,
    user_email: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct SameEmailArgs {
    candidate_email: Option<String>,
    user_id: Option<String>,
    user_email: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct AddressesMemberArgs {
    member: Box<Member>,
    user_id: Option<String>,
    user_email: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct AddressesUserArgs {
    user: Box<LiteLLMUserTable>,
    user_id: Option<String>,
    user_email: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct WithRowEmailArgs {
    user_id: Option<String>,
    user_email: Option<String>,
    email_of: BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct RawIdentity {
    user_id: Option<String>,
    user_email: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "validate")]
    Validate(ValidateArgs),
    #[serde(rename = "same_email")]
    SameEmail(SameEmailArgs),
    #[serde(rename = "addresses_member")]
    AddressesMember(AddressesMemberArgs),
    #[serde(rename = "addresses_user")]
    AddressesUser(AddressesUserArgs),
    #[serde(rename = "with_row_email")]
    WithRowEmail(WithRowEmailArgs),
    #[serde(rename = "duplicate_indexes")]
    DuplicateIndexes { requests: Vec<RawIdentity> },
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
            Input::Validate(args) => run_validate(args),
            Input::SameEmail(args) => run_same_email(args),
            Input::AddressesMember(args) => run_addresses_member(args),
            Input::AddressesUser(args) => run_addresses_user(args),
            Input::WithRowEmail(args) => run_with_row_email(args),
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

fn run_validate(args: &ValidateArgs) -> crate::helpers::HelperResult {
    let identity = build_identity(args.user_id.as_deref(), args.user_email.as_deref())?;
    to_json(&identity)
}

fn run_same_email(args: &SameEmailArgs) -> crate::helpers::HelperResult {
    let request = build_identity(args.user_id.as_deref(), args.user_email.as_deref())?;
    to_json(&same_email(args.candidate_email.as_deref(), &request))
}

#[derive(Clone, Copy)]
enum AddressTarget<'row> {
    Member(&'row Member),
    User(&'row LiteLLMUserTable),
}

fn run_addresses(
    target: AddressTarget<'_>,
    user_id: Option<&str>,
    user_email: Option<&str>,
) -> crate::helpers::HelperResult {
    let request = build_identity(user_id, user_email)?;
    let matched = match target {
        AddressTarget::Member(member) => addresses_member(member, &request),
        AddressTarget::User(user) => addresses_user(user, &request),
    };
    to_json(&matched)
}

fn run_addresses_member(args: &AddressesMemberArgs) -> crate::helpers::HelperResult {
    let target = AddressTarget::Member(&args.member);
    run_addresses(target, args.user_id.as_deref(), args.user_email.as_deref())
}

fn run_addresses_user(args: &AddressesUserArgs) -> crate::helpers::HelperResult {
    let target = AddressTarget::User(&args.user);
    run_addresses(target, args.user_id.as_deref(), args.user_email.as_deref())
}

fn run_with_row_email(args: &WithRowEmailArgs) -> crate::helpers::HelperResult {
    let request = build_identity(args.user_id.as_deref(), args.user_email.as_deref())?;
    to_json(&with_row_email(request, &args.email_of))
}

fn run_duplicate_indexes(requests: &[RawIdentity]) -> crate::helpers::HelperResult {
    let identities = requests
        .iter()
        .map(|raw| build_identity(raw.user_id.as_deref(), raw.user_email.as_deref()))
        .collect::<Result<Vec<MemberIdentity>, _>>()?;
    let indexes = duplicate_member_indexes(&identities);
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
