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
    let _lens_settings = decode::<LensSettings>(bytes);
    let _lens = decode::<Lens>(bytes);
    let _lens_list = decode::<LensList>(bytes);
    let _job = decode::<Job>(bytes);
    let _worker = decode::<Worker>(bytes);
    let _claim = decode::<Claim>(bytes);
    let _job_result = decode::<JobResult>(bytes);
    let _progress = decode::<Progress>(bytes);
    let _run_request = decode::<RunRequest>(bytes);
    let _finding_update = decode::<FindingUpdate>(bytes);
    let _model_request = decode::<ModelRequest>(bytes);
    let _model_result = decode::<ModelResult>(bytes);
    let _execution_content = decode::<ExecutionContent>(bytes);
    let _sample = decode::<Sample>(bytes);
    let _evidence = decode::<Evidence>(bytes);
    let _finding_draft = decode::<FindingDraft>(bytes);
});
