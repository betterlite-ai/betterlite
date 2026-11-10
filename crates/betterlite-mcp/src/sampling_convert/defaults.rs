use serde::Deserialize;
use serde::Deserializer;

// MCP declares these fields nullable with a default, so null takes the same default as an absent key
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let Some(value) = Option::<T>::deserialize(deserializer)? else {
        return Ok(T::default());
    };
    Ok(value)
}
