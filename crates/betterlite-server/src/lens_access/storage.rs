use betterlite_lens::sources::source_reader::SourceReader;
use betterlite_lens::sources::storage::Storage;

use crate::lens_access::error::LensAccessError;

pub(crate) fn source_reader<Store>(
    storage: Option<&Store>,
) -> Result<SourceReader<Store>, LensAccessError>
where
    Store: Storage + Clone,
{
    storage
        .map(|found| SourceReader {
            storage: found.clone(),
        })
        .ok_or(LensAccessError::TracingDisabled)
}

mod tests;
