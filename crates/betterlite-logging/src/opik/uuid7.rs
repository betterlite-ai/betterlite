use chrono::Utc;
use uuid::Uuid;

#[must_use]
pub fn create_uuid7() -> String {
    let timestamp = Utc::now().timestamp_millis();
    let timestamp_bytes = timestamp.to_be_bytes();
    let mut bytes = Uuid::new_v4().into_bytes();
    bytes[..6].copy_from_slice(&timestamp_bytes[2..]);
    bytes[6] = 0x70 | (bytes[6] & 0x0f);
    bytes[8] = 0x80 | (bytes[8] & 0x3f);
    Uuid::from_bytes(bytes).to_string()
}
