mod effects;
mod input;
mod job;
mod logging;

use std::error::Error;

use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::input::Input;
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(&error))?;
        let jobs = input
            .jobs
            .iter()
            .map(|job| job::run_job(job, &runtime))
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(json!({"jobs": jobs}))
    }
}

pub(crate) fn failure(error: &dyn Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BflJobsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
