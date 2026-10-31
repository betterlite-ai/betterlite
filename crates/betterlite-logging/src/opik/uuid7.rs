use chrono::Utc;
use uuid::{Builder, Uuid, Variant, Version};

#[must_use]
pub fn create_uuid7() -> String {
    let timestamp = Utc::now().timestamp_millis();
    let timestamp_bytes = timestamp.to_be_bytes();
    let mut bytes = Uuid::new_v4().into_bytes();
    bytes[..6].copy_from_slice(&timestamp_bytes[2..]);
    Builder::from_bytes(bytes)
        .with_version(Version::SortRand)
        .with_variant(Variant::RFC4122)
        .into_uuid()
        .to_string()
}
