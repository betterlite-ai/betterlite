#![no_main]

use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerObject,
};
use betterlite_openai::containers::replies::decode_container_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _container = decode_container_reply::<ContainerObject>(bytes);
    let _file_list = decode_container_reply::<ContainerFileListResponse>(bytes);
});
