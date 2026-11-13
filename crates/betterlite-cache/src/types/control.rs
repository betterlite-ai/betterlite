use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicCacheControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, rename = "s-maxage", skip_serializing_if = "Option::is_none")]
    pub s_maxage: Option<i64>,
    #[serde(default, rename = "s-max-age", skip_serializing_if = "Option::is_none")]
    pub s_max_age: Option<i64>,
    #[serde(default, rename = "no-cache", skip_serializing_if = "Option::is_none")]
    pub no_cache: Option<bool>,
    #[serde(default, rename = "no-store", skip_serializing_if = "Option::is_none")]
    pub no_store: Option<bool>,
}
