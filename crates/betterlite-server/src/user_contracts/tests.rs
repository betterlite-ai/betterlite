#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{MAX_JSON_BYTES, MAX_JSON_DEPTH};
use rstest::rstest;

use crate::proxy_request_contracts::ContractError;
use crate::user_contracts::bulk_delete::{
    BulkDeleteUserRequest, BulkDeleteUsersResponse, MAX_BULK_DELETE_USERS, UserDeleteResult,
};
use crate::user_contracts::bulk_new::{
    BulkNewUserItem, BulkNewUserMeta, BulkNewUserRequest, BulkNewUserResponse, MAX_BULK_NEW_USERS,
    UserCreateResult,
};
use crate::user_contracts::bulk_update::{
    BulkUpdateUserRequest, BulkUpdateUserResponse, UserUpdateResult,
};
use crate::user_contracts::error::UserContractsError;
use crate::user_contracts::list::{LiteLLMUserTableWithKeyCount, UserListResponse};
use crate::user_contracts::query::{MAX_USER_ID_LENGTH, is_valid_user_id, user_id_from_query};
use crate::user_contracts::search::{UserSearchTerm, UserSearchWhere};
use crate::user_contracts::{decode_bulk_user_request, decode_user_info_query};

const SELECTION_MISSING: &str = "Must specify either 'users' for individual updates or 'all_users=True' with 'user_updates' for bulk updates";
const SELECTION_CONFLICT: &str =
    "Cannot specify both 'users' and 'all_users=True'. Choose one approach.";

fn contract(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("a JSON fixture")
}

fn update(text: &str) -> Result<BulkUpdateUserRequest, UserContractsError> {
    decode_bulk_user_request(text.as_bytes(), BulkUpdateUserRequest::from_value)
}

fn delete(text: &str) -> Result<BulkDeleteUserRequest, UserContractsError> {
    decode_bulk_user_request(text.as_bytes(), BulkDeleteUserRequest::from_value)
}

fn new_users(text: &str) -> Result<BulkNewUserRequest, UserContractsError> {
    decode_bulk_user_request(text.as_bytes(), BulkNewUserRequest::from_value)
}

fn validation_text(error: &ContractError) -> String {
    match error {
        ContractError::Validation { message } => message.clone(),
        ContractError::Decode { source } => panic!("a decode rejection: {source}"),
    }
}

fn rejection_text(error: &UserContractsError) -> String {
    match error {
        UserContractsError::Contract(inner) => validation_text(inner),
        UserContractsError::Bounded(reason) => panic!("a bounded rejection: {reason}"),
    }
}

fn assert_contract_decode_rejected(error: &ContractError) {
    match error {
        ContractError::Decode { .. } => {}
        ContractError::Validation { message } => panic!("a decode rejection, got {message}"),
    }
}

fn assert_decode_rejected(error: &UserContractsError) {
    match error {
        UserContractsError::Contract(inner) => assert_contract_decode_rejected(inner),
        UserContractsError::Bounded(reason) => panic!("a bounded rejection: {reason}"),
    }
}

fn user_ids(count: usize) -> String {
    let ids: Vec<String> = (0..count).map(|index| format!("\"user{index}\"")).collect();
    format!("{{\"user_ids\": [{}]}}", ids.join(", "))
}

fn new_user_rows(count: usize) -> String {
    let rows: Vec<String> = (0..count)
        .map(|index| format!("{{\"user_id\": \"user{index}\"}}"))
        .collect();
    format!("{{\"users\": [{}]}}", rows.join(", "))
}

#[test]
fn update_defaults() {
    let request = update("{}").expect("an empty bulk update");
    assert!(request.users.is_none());
    assert_eq!(request.all_users, Some(false));
    assert!(request.user_updates.is_none());
}

#[test]
fn update_explicit_null() {
    let all_null = update(r#"{"users": null, "all_users": null, "user_updates": null}"#)
        .expect_err("an explicit null user_updates still runs the validator");
    assert_eq!(rejection_text(&all_null), SELECTION_MISSING);
    let null_updates = update(r#"{"all_users": true, "user_updates": null}"#)
        .expect_err("a null user_updates is not truthy");
    assert_eq!(rejection_text(&null_updates), SELECTION_MISSING);
    let null_users = update(r#"{"users": null, "all_users": true, "user_updates": {}}"#)
        .expect("an explicit null users list is absent");
    assert!(null_users.users.is_none());
    assert_eq!(null_users.all_users, Some(true));
    assert!(null_users.user_updates.is_some());
}

#[test]
fn update_valid_selection() {
    let listed = update(r#"{"users": [{"user_id": "alice", "user_role": "internal_user"}]}"#)
        .expect("a users list alone is enough");
    assert_eq!(listed.all_users, Some(false));
    let users = listed.users.expect("the decoded users list");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].user_id.as_deref(), Some("alice"));
    assert_eq!(users[0].fields.user_role, Some(UserRole::InternalUser));
    assert_eq!(
        users[0].fields.supplied,
        BTreeSet::from(["user_role".to_owned()])
    );
    let everyone = update(r#"{"all_users": true, "user_updates": {"max_budget": 5}}"#)
        .expect("all_users with updates");
    assert!(everyone.users.is_none());
    assert_eq!(everyone.all_users, Some(true));
    let updates = everyone.user_updates.expect("the decoded updates");
    assert_eq!(updates.supplied, BTreeSet::from(["max_budget".to_owned()]));
    let empty_updates = update(r#"{"all_users": true, "user_updates": {}}"#)
        .expect("an empty updates model is truthy");
    assert_eq!(
        empty_updates
            .user_updates
            .expect("the decoded updates")
            .supplied,
        BTreeSet::new()
    );
}

#[test]
fn update_present_field_conflict() {
    let both =
        update(r#"{"users": [{"user_id": "alice"}], "all_users": true, "user_updates": {}}"#)
            .expect_err("users and all_users together conflict");
    assert_eq!(rejection_text(&both), SELECTION_CONFLICT);
    let null_updates =
        update(r#"{"users": [{"user_id": "alice"}], "all_users": true, "user_updates": null}"#)
            .expect_err("the conflict check runs before the missing check");
    assert_eq!(rejection_text(&null_updates), SELECTION_CONFLICT);
}

#[test]
fn update_omitted_field_skips_validation() {
    let omitted = update(r#"{"users": [{"user_id": "alice"}], "all_users": true}"#)
        .expect("without user_updates the combination validator never runs");
    assert_eq!(omitted.all_users, Some(true));
    assert_eq!(omitted.users.expect("the users list").len(), 1);
    let empty = update(r#"{"users": [], "all_users": false}"#)
        .expect("empty defaults alone reach the mutation service");
    assert_eq!(empty.users.expect("the empty users list").len(), 0);
    let missing = update(r#"{"users": [], "all_users": false, "user_updates": {}}"#)
        .expect_err("a supplied user_updates runs the validator");
    assert_eq!(rejection_text(&missing), SELECTION_MISSING);
    let all = update(r#"{"users": [], "all_users": true, "user_updates": {}}"#)
        .expect("all_users with truthy updates satisfies the validator");
    assert_eq!(all.all_users, Some(true));
}

#[test]
fn update_response_defaults() {
    let result: UserUpdateResult =
        serde_json::from_str(r#"{"success": true, "ignored": 1}"#).expect("a result row");
    assert_eq!(result.user_id, None);
    assert_eq!(result.user_email, None);
    assert!(result.success);
    assert_eq!(result.error, None);
    assert_eq!(result.updated_user, None);
    assert!(serde_json::from_str::<UserUpdateResult>(r#"{"user_id": "u"}"#).is_err());
    let response: BulkUpdateUserResponse = serde_json::from_str(
        r#"{"results": [{"success": true, "user_id": "u"}], "total_requested": 1, "successful_updates": 1, "failed_updates": 0}"#,
    )
    .expect("a bulk update response");
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.total_requested, 1);
    assert_eq!(response.failed_updates, 0);
}

#[test]
fn delete_bounds_and_extras() {
    assert_eq!(MAX_BULK_DELETE_USERS, 500);
    let empty = delete(r#"{"user_ids": []}"#).expect_err("an empty list is rejected");
    assert_eq!(
        rejection_text(&empty),
        "user_ids must hold between 1 and 500 values"
    );
    let full = delete(&user_ids(MAX_BULK_DELETE_USERS)).expect("500 ids are allowed");
    assert_eq!(full.user_ids.len(), MAX_BULK_DELETE_USERS);
    let over = delete(&user_ids(MAX_BULK_DELETE_USERS + 1)).expect_err("501 ids are rejected");
    assert_eq!(
        rejection_text(&over),
        "user_ids must hold between 1 and 500 values"
    );
    let extras =
        delete(r#"{"user_ids": ["u"], "skip_teams": true}"#).expect_err("an extra key is rejected");
    assert_decode_rejected(&extras);
    let wrong_type = delete(r#"{"user_ids": "u"}"#).expect_err("a string is not a tuple");
    assert_decode_rejected(&wrong_type);
    let non_string = delete(r#"{"user_ids": [1]}"#).expect_err("a number is not an id");
    assert_decode_rejected(&non_string);
    let missing = delete(r#"{}"#).expect_err("user_ids is required");
    assert_decode_rejected(&missing);
}

#[test]
fn delete_duplicates_preserved() {
    let request = delete(r#"{"user_ids": ["b", "a", "a", " c "]}"#).expect("a delete request");
    assert_eq!(
        request.user_ids,
        vec![
            "b".to_owned(),
            "a".to_owned(),
            "a".to_owned(),
            " c ".to_owned()
        ]
    );
}

#[test]
fn delete_response_empty_teams() {
    let result: UserDeleteResult =
        serde_json::from_str(r#"{"user_id": "u", "success": false}"#).expect("a delete result");
    assert_eq!(result.user_email, None);
    assert_eq!(result.teams_removed, Vec::<String>::new());
    assert_eq!(result.error, None);
    let removed: UserDeleteResult =
        serde_json::from_str(r#"{"user_id": "u", "success": true, "teams_removed": ["t"]}"#)
            .expect("a result that left a team");
    assert_eq!(removed.teams_removed, vec!["t".to_owned()]);
    let envelope: BulkDeleteUsersResponse =
        serde_json::from_str(r#"{"data": [{"user_id": "u", "success": true}], "meta": {}}"#)
            .expect("the data envelope ignores extra fields");
    assert_eq!(envelope.data.len(), 1);
    assert_eq!(envelope.data[0].user_id, "u");
}

#[test]
fn new_key_opt_in() {
    let opt_out =
        BulkNewUserItem::from_value(&contract(r#"{"user_id": "alice"}"#)).expect("an item");
    assert!(!opt_out.request.auto_create_key);
    assert_eq!(opt_out.request.user_role, None);
    assert!(opt_out.request.teams.is_none());
    assert_eq!(opt_out.request.send_invite_email, None);
    let opt_in = BulkNewUserItem::from_value(&contract(
        r#"{"user_id": "alice", "auto_create_key": true}"#,
    ))
    .expect("an item that asks for a key");
    assert!(opt_in.request.auto_create_key);
    let explicit_off = BulkNewUserItem::from_value(&contract(
        r#"{"user_id": "alice", "auto_create_key": false}"#,
    ))
    .expect("an item that declines a key");
    assert!(!explicit_off.request.auto_create_key);
}

#[test]
fn new_invite_rejected() {
    let invited = BulkNewUserItem::from_value(&contract(
        r#"{"user_id": "alice", "send_invite_email": true}"#,
    ))
    .expect_err("a truthy invite flag is rejected");
    assert_eq!(
        validation_text(&invited),
        "send_invite_email is not supported on /management/v1/users/bulk; invite users separately"
    );
    let declined = BulkNewUserItem::from_value(&contract(
        r#"{"user_id": "alice", "send_invite_email": false}"#,
    ))
    .expect("a false invite flag passes");
    assert_eq!(declined.request.send_invite_email, Some(false));
    let absent = BulkNewUserItem::from_value(&contract(
        r#"{"user_id": "alice", "send_invite_email": null}"#,
    ))
    .expect("a null invite flag passes");
    assert_eq!(absent.request.send_invite_email, None);
}

#[test]
fn new_password_rejected() {
    let set =
        BulkNewUserItem::from_value(&contract(r#"{"user_id": "alice", "password": "hunter2"}"#))
            .expect_err("a non-null password is rejected");
    assert!(validation_text(&set).contains("password cannot be set via /user/new"));
    let empty = BulkNewUserItem::from_value(&contract(r#"{"user_id": "alice", "password": ""}"#))
        .expect_err("an empty password is still non-null");
    assert!(validation_text(&empty).contains("password cannot be set via /user/new"));
    let null = BulkNewUserItem::from_value(&contract(r#"{"user_id": "alice", "password": null}"#))
        .expect("a null password passes");
    assert!(null.request.password.is_none());
}

#[test]
fn new_bounds_and_extras() {
    assert_eq!(MAX_BULK_NEW_USERS, 500);
    let empty = new_users(r#"{"users": []}"#).expect_err("an empty batch is rejected");
    assert_eq!(
        rejection_text(&empty),
        "users must hold between 1 and 500 values"
    );
    let single = new_users(r#"{"users": [{}]}"#).expect("user_id is optional on each row");
    assert_eq!(single.users.len(), 1);
    assert!(single.users[0].request.user_id.is_none());
    let full = new_users(&new_user_rows(MAX_BULK_NEW_USERS)).expect("500 rows are allowed");
    assert_eq!(full.users.len(), MAX_BULK_NEW_USERS);
    let over =
        new_users(&new_user_rows(MAX_BULK_NEW_USERS + 1)).expect_err("501 rows are rejected");
    assert_eq!(
        rejection_text(&over),
        "users must hold between 1 and 500 values"
    );
    let extra_top =
        new_users(r#"{"users": [{"user_id": "u"}], "all_users": true}"#).expect_err("an extra key");
    assert_eq!(
        rejection_text(&extra_top),
        "extra inputs are not permitted: `all_users`"
    );
    let extra_row = new_users(r#"{"users": [{"user_id": "u", "nickname": "n"}]}"#)
        .expect_err("an extra row key");
    assert_eq!(
        rejection_text(&extra_row),
        "extra inputs are not permitted: `nickname`"
    );
    let missing = new_users(r#"{}"#).expect_err("users is required");
    assert_decode_rejected(&missing);
    let wrong_type = new_users(r#"{"users": "u"}"#).expect_err("a string is not a list");
    assert_decode_rejected(&wrong_type);
    let non_object = new_users(r#"{"users": [5]}"#).expect_err("a number is not a row");
    assert_decode_rejected(&non_object);
    let blank_budget = new_users(r#"{"users": [{"user_id": "u", "max_budget": ""}]}"#)
        .expect("an empty max_budget becomes null");
    assert!(blank_budget.users[0].request.max_budget.is_none());
}

#[test]
fn new_response_nulls() {
    let result: UserCreateResult =
        serde_json::from_str(r#"{"success": false, "extra": true}"#).expect("a create result");
    assert_eq!(result.user_id, None);
    assert_eq!(result.user_email, None);
    assert_eq!(result.teams, None);
    assert_eq!(result.key, None);
    assert_eq!(result.error, None);
    let empty_teams: UserCreateResult =
        serde_json::from_str(r#"{"success": true, "teams": []}"#).expect("empty teams");
    assert_eq!(empty_teams.teams, Some(Vec::new()));
    let meta: BulkNewUserMeta =
        serde_json::from_str(r#"{"total_requested": 2, "created": 1, "failed": 1}"#)
            .expect("a meta block");
    assert_eq!(meta.total_requested, 2);
    let response: BulkNewUserResponse = serde_json::from_str(
        r#"{"data": [{"success": true}], "meta": {"total_requested": 1, "created": 1, "failed": 0}}"#,
    )
    .expect("a bulk new response");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.meta.created, 1);
}

#[test]
fn id_character_boundaries() {
    assert!(is_valid_user_id(""));
    assert!(is_valid_user_id(&"a".repeat(MAX_USER_ID_LENGTH)));
    assert!(!is_valid_user_id(&"a".repeat(MAX_USER_ID_LENGTH + 1)));
    assert!(is_valid_user_id(&"é".repeat(MAX_USER_ID_LENGTH)));
    assert!(!is_valid_user_id(&"é".repeat(MAX_USER_ID_LENGTH + 1)));
    assert!(is_valid_user_id("\u{7f}"));
    assert!(is_valid_user_id("with space"));
    assert!(!is_valid_user_id("\u{0}"));
    assert!(!is_valid_user_id("\u{1f}"));
    assert!(user_id_from_query(&format!("user_id={}", "a".repeat(512))).is_some());
    assert!(user_id_from_query(&format!("user_id={}", "a".repeat(513))).is_none());
}

#[test]
fn id_plus_preserved() {
    assert_eq!(
        user_id_from_query("tag=1&user_id=a+b"),
        Some("a+b".to_owned())
    );
    assert_eq!(user_id_from_query("user_id=a%2Bb"), Some("a+b".to_owned()));
    assert_eq!(
        decode_user_info_query("user_id=a+b", Some("a b")),
        Some("a+b".to_owned())
    );
}

#[test]
fn id_first_unanchored_match() {
    assert_eq!(
        user_id_from_query("other_user_id=x&user_id=y"),
        Some("x".to_owned())
    );
    assert_eq!(
        user_id_from_query("user_id=a&user_id=b"),
        Some("a".to_owned())
    );
    assert_eq!(user_id_from_query("tag=1&user_id=z"), Some("z".to_owned()));
    assert_eq!(user_id_from_query("user_id="), Some(String::new()));
    assert_eq!(user_id_from_query("no_identity=1"), None);
}

#[test]
fn id_percent_decode_once() {
    assert_eq!(
        user_id_from_query("user_id=a%2520b"),
        Some("a%20b".to_owned())
    );
    assert_eq!(user_id_from_query("user_id=a%20b"), Some("a b".to_owned()));
    assert_eq!(user_id_from_query("user_id=%2B"), Some("+".to_owned()));
    assert_eq!(user_id_from_query("user_id=%zz"), Some("%zz".to_owned()));
    assert_eq!(user_id_from_query("user_id=%"), Some("%".to_owned()));
}

#[test]
fn id_invalid_utf8_replacement() {
    assert_eq!(
        user_id_from_query("user_id=%FF"),
        Some("\u{fffd}".to_owned())
    );
    assert_eq!(
        user_id_from_query("user_id=%C3%28"),
        Some("\u{fffd}(".to_owned())
    );
}

#[test]
fn id_controls_null() {
    assert_eq!(user_id_from_query("user_id=%00"), None);
    assert_eq!(user_id_from_query("user_id=a%1Fb"), None);
    assert_eq!(user_id_from_query("user_id=a%09b"), None);
    assert_eq!(user_id_from_query("user_id=%7F"), Some("\u{7f}".to_owned()));
    assert_eq!(user_id_from_query("user_id=%20"), Some(" ".to_owned()));
}

#[test]
fn id_normalization_only_for_spaces() {
    assert_eq!(
        decode_user_info_query("user_id=a+b", Some("a b")),
        Some("a+b".to_owned())
    );
    assert_eq!(
        decode_user_info_query("user_id=zzz", Some("a b")),
        Some("zzz".to_owned())
    );
    assert_eq!(decode_user_info_query("nothing_here=1", Some("a b")), None);
    assert_eq!(
        decode_user_info_query("user_id=a%20b", Some("a\u{1}")),
        Some("a\u{1}".to_owned())
    );
    let overlong = "b".repeat(MAX_USER_ID_LENGTH + 1);
    assert_eq!(
        decode_user_info_query("user_id=c", Some(overlong.as_str())),
        Some(overlong.clone())
    );
    assert_eq!(decode_user_info_query("user_id=c", None), None);
}

#[test]
fn user_contract_input_limits() {
    let mut at_limit = vec![b' '; MAX_JSON_BYTES];
    at_limit[0] = b'{';
    at_limit[1] = b'}';
    drop(
        decode_bulk_user_request(&at_limit, BulkUpdateUserRequest::from_value)
            .expect("a body of exactly the byte limit"),
    );
    let over_limit = vec![b' '; MAX_JSON_BYTES + 1];
    let too_large = decode_bulk_user_request(&over_limit, BulkUpdateUserRequest::from_value)
        .expect_err("one byte over the limit");
    assert!(matches!(
        too_large,
        UserContractsError::Bounded(PrivateJsonError::TooLarge(..))
    ));
    let mut at_depth = String::from(r#"{"ignored":"#);
    at_depth.push_str(&"[".repeat(MAX_JSON_DEPTH - 1));
    at_depth.push_str(&"]".repeat(MAX_JSON_DEPTH - 1));
    at_depth.push('}');
    drop(
        decode_bulk_user_request(at_depth.as_bytes(), BulkUpdateUserRequest::from_value)
            .expect("a body exactly at the depth limit"),
    );
    let mut too_deep = String::from(r#"{"ignored":"#);
    too_deep.push_str(&"[".repeat(MAX_JSON_DEPTH));
    too_deep.push_str(&"]".repeat(MAX_JSON_DEPTH));
    too_deep.push('}');
    let depth_error =
        decode_bulk_user_request(too_deep.as_bytes(), BulkUpdateUserRequest::from_value)
            .expect_err("one level over the depth limit");
    assert!(matches!(
        depth_error,
        UserContractsError::Bounded(PrivateJsonError::TooDeep(..))
    ));
    let malformed = decode_bulk_user_request(b"{", BulkDeleteUserRequest::from_value)
        .expect_err("a truncated body");
    assert!(matches!(
        malformed,
        UserContractsError::Bounded(PrivateJsonError::Decode(..))
    ));
}

#[test]
fn search_where_and_list_rows() {
    let where_clause = UserSearchWhere::from_value(&contract(
        r#"{"OR": [{"user_id": {"contains": "Al", "mode": "insensitive"}}, {"user_email": {"contains": "ex", "mode": "insensitive"}}]}"#,
    ))
    .expect("a search filter");
    let extra = UserSearchWhere::from_value(&contract(
        r#"{"OR": [{"user_id": {"contains": "Al", "mode": "insensitive"}, "extra": 1}]}"#,
    ))
    .expect_err("the mapping admits only user_id and user_email");
    assert_contract_decode_rejected(&extra);
    assert_eq!(where_clause.or.len(), 2);
    let first: &UserSearchTerm = &where_clause.or[0];
    let contains = first.user_id.as_ref().expect("the user_id term");
    assert_eq!(contains.contains, "Al");
    assert!(first.user_email.is_none());
    assert_eq!(
        where_clause.or[1]
            .user_email
            .as_ref()
            .expect("the user_email term")
            .contains,
        "ex"
    );
    assert!(serde_json::from_str::<UserSearchWhere>(r#"{"OR": []}"#).is_ok());
    let wrong_mode = UserSearchWhere::from_value(&contract(
        r#"{"OR": [{"user_id": {"contains": "Al", "mode": "sensitive"}}]}"#,
    ))
    .expect_err("only the insensitive mode exists");
    assert_contract_decode_rejected(&wrong_mode);
    let null_term = UserSearchWhere::from_value(&contract(r#"{"OR": [{"user_id": null}]}"#))
        .expect_err("a null mapping value is not an absent key");
    assert_contract_decode_rejected(&null_term);
    let junk = UserSearchWhere::from_value(&contract(
        r#"{"OR": [{"user_id": {"contains": "Al", "mode": "insensitive", "junk": 1}}], "AND": []}"#,
    ))
    .expect("extra keys inside a term and at the top are ignored");
    assert_eq!(
        serde_json::to_value(&junk).expect("a serialized filter"),
        contract(r#"{"OR": [{"user_id": {"contains": "Al", "mode": "insensitive"}}]}"#)
    );
    let list: UserListResponse = serde_json::from_str(
        r#"{"users": [{"user_id": "u"}], "total": 1, "page": 1, "page_size": 25, "total_pages": 1}"#,
    )
    .expect("a user list");
    assert_eq!(list.users.len(), 1);
    let row: &LiteLLMUserTableWithKeyCount = &list.users[0];
    assert_eq!(row.user.user_id, "u");
    assert_eq!(row.key_count, 0);
    let keyed: UserListResponse = serde_json::from_str(
        r#"{"users": [{"user_id": "u", "key_count": 3}], "total": 1, "page": 2, "page_size": 25, "total_pages": 1}"#,
    )
    .expect("a row with a key count");
    assert_eq!(keyed.users[0].key_count, 3);
    assert!(serde_json::from_str::<UserListResponse>(r#"{"users": []}"#).is_err());
    let row_value: LiteLLMUserTable = serde_json::from_str(r#"{"user_id": "u", "key_count": 1}"#)
        .expect("a row ignores key_count");
    assert_eq!(row_value.user_id, "u");
}

#[rstest]
fn replays_the_user_contracts_corpus(#[files("fuzz/corpus/user_contracts/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus seed");
    drop(decode_bulk_user_request(
        &bytes,
        BulkUpdateUserRequest::from_value,
    ));
    drop(decode_bulk_user_request(
        &bytes,
        BulkDeleteUserRequest::from_value,
    ));
    drop(decode_bulk_user_request(
        &bytes,
        BulkNewUserRequest::from_value,
    ));
}
