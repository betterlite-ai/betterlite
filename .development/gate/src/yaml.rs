use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::error::GateError;

pub(crate) fn read_yaml<Spec>(path: &Path) -> Result<Spec, GateError>
where
    Spec: DeserializeOwned,
{
    let text = fs::read_to_string(path).map_err(|source| GateError::Read {
        path: path.to_owned(),
        source,
    })?;
    let strict = serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            max_total_comment_bytes: 0,
            max_anchors: 0,
            max_aliases: 0,
            max_documents: 1,
        },
    };
    serde_saphyr::from_str_with_options(&text, strict).map_err(|source| GateError::Yaml {
        path: path.to_owned(),
        source: Box::new(source),
    })
}
