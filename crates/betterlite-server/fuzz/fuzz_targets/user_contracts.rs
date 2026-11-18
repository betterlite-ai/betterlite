#![no_main]

use betterlite_server::user_contracts::bulk_delete::BulkDeleteUserRequest;
use betterlite_server::user_contracts::bulk_new::BulkNewUserRequest;
use betterlite_server::user_contracts::bulk_update::BulkUpdateUserRequest;
use betterlite_server::user_contracts::{decode_bulk_user_request, decode_user_info_query};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_bulk_user_request(
        bytes,
        BulkUpdateUserRequest::from_value,
    ));
    drop(decode_bulk_user_request(
        bytes,
        BulkDeleteUserRequest::from_value,
    ));
    drop(decode_bulk_user_request(
        bytes,
        BulkNewUserRequest::from_value,
    ));
    if let Ok(query) = std::str::from_utf8(bytes) {
        drop(decode_user_info_query(query, Some(query)));
    }
});
