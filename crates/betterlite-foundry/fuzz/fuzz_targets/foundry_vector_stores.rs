#![no_main]

use betterlite_foundry::vector_stores::reply::{AzureSearchStoreReply, decode_azure_search_reply};
use betterlite_foundry::vector_stores::search::SearchLogging;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let logging = SearchLogging::default();
    let reply = AzureSearchStoreReply {
        status_code: 200,
        headers: &[],
        body: bytes,
        logging: &logging,
    };
    let _decoded = decode_azure_search_reply(&reply);
});
