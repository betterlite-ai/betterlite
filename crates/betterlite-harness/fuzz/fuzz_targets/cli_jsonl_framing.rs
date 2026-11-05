#![no_main]

use betterlite_harness::cli_handler::io::iter_stream_lines;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

fuzz_target!(|input: &[u8]| {
    if let Ok(runtime) = Builder::new_current_thread().build() {
        runtime.block_on(async {
            let _outcome = iter_stream_lines(input).await;
        });
    }
});
