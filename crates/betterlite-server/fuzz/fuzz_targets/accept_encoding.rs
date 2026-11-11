#![no_main]
use betterlite_server::middleware::gzip_buffered::accepts_gzip;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|header: &[u8]| {
    let _accepted = accepts_gzip(header);
});
