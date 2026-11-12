#![no_main]

use betterlite_provider::manus_files::reply::{
    decode_create_file, decode_deleted_file, decode_file_list,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _create = decode_create_file(input, 1_700_000_000);
    let _list = decode_file_list(input, 1_700_000_000);
    let _deleted = decode_deleted_file(input);
});
