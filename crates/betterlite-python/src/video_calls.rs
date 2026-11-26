use pyo3::prelude::*;

mod adapter;
mod clients;
mod convert;
mod error;
mod session;

pub const fn register(_module: &Bound<'_, PyModule>) -> PyResult<()> {
    // The video_calls family task adds its native functions here.
    Ok(())
}
