use std::error::Error;
use std::fmt;
use std::future::Future;

use betterlite_config::sections::mcp_toolset_storage::MCPToolset;
use serde::de::value::MapAccessDeserializer;
use serde::de::{Error as SerdeError, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod invalidate;
pub mod lookup;
mod tests;

// LiteLLM caches a missing toolset under the bare string "__not_found__", not a tagged variant
const NOT_FOUND_SENTINEL: &str = "__not_found__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedToolset {
    Found(MCPToolset),
    NotFound,
}

// ast-grep-ignore: no-custom-serde
impl Serialize for CachedToolset {
    // ast-grep-ignore: no-custom-serde
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CachedToolset::Found(toolset) => toolset.serialize(serializer),
            CachedToolset::NotFound => serializer.serialize_str(NOT_FOUND_SENTINEL),
        }
    }
}

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for CachedToolset {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CachedToolsetVisitor;

        // ast-grep-ignore: no-custom-serde
        impl<'de> Visitor<'de> for CachedToolsetVisitor {
            type Value = CachedToolset;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a toolset object or the __not_found__ string")
            }

            fn visit_str<E>(self, text: &str) -> Result<CachedToolset, E>
            where
                E: SerdeError,
            {
                if text == NOT_FOUND_SENTINEL {
                    return Ok(CachedToolset::NotFound);
                }
                Err(E::custom(format!(
                    "the cache value {text:?} is not a toolset"
                )))
            }

            fn visit_map<A>(self, map: A) -> Result<CachedToolset, A::Error>
            where
                A: MapAccess<'de>,
            {
                MCPToolset::deserialize(MapAccessDeserializer::new(map)).map(CachedToolset::Found)
            }
        }

        deserializer.deserialize_any(CachedToolsetVisitor)
    }
}

pub trait ToolsetCache: Send + Sync {
    type Error: Error + 'static;

    fn get(&self, key: &str) -> impl Future<Output = Option<CachedToolset>> + Send;

    fn set(
        &self,
        key: &str,
        value: &CachedToolset,
        ttl_secs: f64,
    ) -> impl Future<Output = ()> + Send;

    fn remove_where(&self, matches: &dyn Fn(&str) -> bool) -> Result<(), Self::Error>;

    fn management_object_ttl(&self) -> f64;
}
