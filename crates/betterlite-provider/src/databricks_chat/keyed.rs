use serde::{Deserialize, Deserializer};

use crate::databricks_chat::error::DatabricksChatError;

// A key LiteLLM reads with [] or tests with "in", where an absent key and a null value differ
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Keyed<T> {
    Missing,
    Null,
    Given(T),
}

impl<T> Keyed<T> {
    // chunk_parser's chunk[key]: an absent key is the 400 KeyError naming the key and the chunk
    pub(crate) fn required(
        self,
        key: &'static str,
        chunk: &str,
    ) -> Result<Option<T>, DatabricksChatError> {
        match self {
            Self::Given(value) => Ok(Some(value)),
            Self::Null => Ok(None),
            Self::Missing => Err(DatabricksChatError::ChunkKey {
                key,
                chunk: chunk.to_owned(),
            }),
        }
    }
}

impl<T> Default for Keyed<T> {
    fn default() -> Self {
        Self::Missing
    }
}

impl<'de, T> Deserialize<'de> for Keyed<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Self::Null, Self::Given))
    }
}
