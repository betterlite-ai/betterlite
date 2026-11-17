use serde::Serialize;
use serde::de::DeserializeOwned;

// Both JSON trees carry the same shapes, so one serde pass moves between them either way.
pub(super) fn convert<Source, Target>(source: &Source) -> Result<Target, serde_json::Error>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let encoded = serde_json::to_value(source)?;
    serde_json::from_value(encoded)
}
