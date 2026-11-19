#![no_main]

use std::str::from_utf8;

use betterlite_server::spend::log_query::decode_spend_log_query;
use betterlite_server::spend::log_query::prepare::SpendLogRoute;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(raw) = from_utf8(bytes) else {
        return;
    };
    let query = decode_spend_log_query(Some(raw));
    drop(query.choices());
    drop(query.prepare(SpendLogRoute::Ui));
    drop(query.prepare(SpendLogRoute::PublicV2));
    let page = query.page.page;
    let page_size = query.page.page_size;
    let encoded = format!("page={page}&page_size={page_size}");
    let again = decode_spend_log_query(Some(&encoded));
    assert_eq!(again.page.page, page);
    assert_eq!(again.page.page_size, page_size);
    assert!(again.framework_issues().is_none());
});
