#![no_main]

use betterlite_pyhost::instance_ref::InstanceRef;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    drop(InstanceRef::parse(value));
});
