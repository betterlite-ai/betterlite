use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::definition::{DefinitionError, RawDefinition, RouteDefinition};
use super::error::RouteLoadError;

pub const MAX_ROUTES_FILE_BYTES: usize = 0x0010_0000;
// one byte past the bound, so a file at the limit reads whole while a longer one still proves TooLarge
const READ_LIMIT_BYTES: u64 = 0x0010_0001;

#[derive(Debug, thiserror::Error)]
pub enum CustomRoutesError {
    #[error("the custom routes input is over the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("the custom routes input is not valid yaml: {source}")]
    Yaml {
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error(
        "the custom routes input must be a list of route definitions or one route definition mapping"
    )]
    InvalidFormat,
    #[error("route definition {index} is invalid: {source}")]
    Definition {
        index: usize,
        #[source]
        source: DefinitionError,
    },
}

pub fn decode(bytes: &[u8]) -> Result<Vec<RouteDefinition>, CustomRoutesError> {
    if bytes.len() > MAX_ROUTES_FILE_BYTES {
        return Err(CustomRoutesError::TooLarge {
            limit: MAX_ROUTES_FILE_BYTES,
        });
    }
    // the document is probed once for its shape, then parsed again as the routes it holds
    let document: GuardrailValue =
        serde_saphyr::from_slice(bytes).map_err(|source| CustomRoutesError::Yaml {
            source: Box::new(source),
        })?;
    match document {
        GuardrailValue::Array(_) => {
            let raw: Vec<RawDefinition> =
                serde_saphyr::from_slice(bytes).map_err(|source| CustomRoutesError::Yaml {
                    source: Box::new(source),
                })?;
            definitions(raw)
        }
        GuardrailValue::Object(_) => {
            let raw: RawDefinition =
                serde_saphyr::from_slice(bytes).map_err(|source| CustomRoutesError::Yaml {
                    source: Box::new(source),
                })?;
            definitions(vec![raw])
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => Err(CustomRoutesError::InvalidFormat),
    }
}

pub fn load(path: &Path) -> Result<Vec<RouteDefinition>, RouteLoadError> {
    let file = File::open(path).map_err(|source| read_error(path, source))?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|source| read_error(path, source))?;
    decode(&bytes).map_err(|source| RouteLoadError::CustomRoutes {
        path: path.to_owned(),
        source,
    })
}

fn read_error(path: &Path, source: io::Error) -> RouteLoadError {
    if source.kind() == io::ErrorKind::NotFound {
        return RouteLoadError::FileNotFound {
            path: path.to_owned(),
            source,
        };
    }
    RouteLoadError::Read {
        path: path.to_owned(),
        source,
    }
}

fn definitions(raw: Vec<RawDefinition>) -> Result<Vec<RouteDefinition>, CustomRoutesError> {
    let mut parsed = Vec::with_capacity(raw.len());
    for (index, item) in raw.into_iter().enumerate() {
        parsed.push(
            item.into_definition()
                .map_err(|source| CustomRoutesError::Definition { index, source })?,
        );
    }
    Ok(parsed)
}
