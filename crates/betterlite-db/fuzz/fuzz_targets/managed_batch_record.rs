#![no_main]

use betterlite_db::auxiliary_repository::decoder::decode_managed_batch_record;
use betterlite_openai::batches::records::BatchObject;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_managed_batch_record::<BatchObject>(data));
});
