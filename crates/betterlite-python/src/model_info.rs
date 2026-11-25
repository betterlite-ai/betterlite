use betterlite_models::fetch_policy::revision::git_blob_id as blob_id;
use pyo3::prelude::*;
mod convert;
mod error;
mod fetch;
mod lookup;
mod map;
mod native;
mod reload;
mod state;
#[pyfunction]
// ast-grep-ignore: no-one-line-functions - a pyfunction is a boundary Python calls by name
pub(crate) fn git_blob_id(body: &[u8]) -> String {
    blob_id(body)
}
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<native::NativeCatalog>()?;
    module.add(
        "ModelLookupError",
        module.py().get_type::<error::ModelLookupError>(),
    )?;
    module.add_function(wrap_pyfunction!(git_blob_id, module)?)?;
    Ok(())
}
