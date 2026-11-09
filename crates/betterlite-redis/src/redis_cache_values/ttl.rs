use num_traits::ToPrimitive as _;

use super::error::RedisCacheValuesError;

// BaseCache.get_ttl takes the given TTL when there is one, cut to an integer toward zero, else the default.
pub(crate) fn resolve_ttl(
    ttl: Option<f64>,
    default_ttl: Option<i64>,
) -> Result<Option<i64>, RedisCacheValuesError> {
    ttl.map_or(Ok(default_ttl), |seconds| {
        seconds
            .trunc()
            .to_i64()
            .map(Some)
            .ok_or(RedisCacheValuesError::TtlOutOfRange { seconds })
    })
}
