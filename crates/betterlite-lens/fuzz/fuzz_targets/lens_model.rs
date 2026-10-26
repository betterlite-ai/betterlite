#![no_main]

use betterlite_lens::models::decode::decode;
use betterlite_lens::models::execution::{ExecutionContent, Sample};
use betterlite_lens::models::finding::{Evidence, FindingDraft};
use betterlite_lens::models::job::{Job, Lens, LensList, Worker};
use betterlite_lens::models::messages::{
    Claim, FindingUpdate, JobResult, ModelRequest, ModelResult, Progress, RunRequest,
};
use betterlite_lens::models::settings::LensSettings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode::<LensSettings>(bytes);
    let _ = decode::<Lens>(bytes);
    let _ = decode::<LensList>(bytes);
    let _ = decode::<Job>(bytes);
    let _ = decode::<Worker>(bytes);
    let _ = decode::<Claim>(bytes);
    let _ = decode::<JobResult>(bytes);
    let _ = decode::<Progress>(bytes);
    let _ = decode::<RunRequest>(bytes);
    let _ = decode::<FindingUpdate>(bytes);
    let _ = decode::<ModelRequest>(bytes);
    let _ = decode::<ModelResult>(bytes);
    let _ = decode::<ExecutionContent>(bytes);
    let _ = decode::<Sample>(bytes);
    let _ = decode::<Evidence>(bytes);
    let _ = decode::<FindingDraft>(bytes);
});
