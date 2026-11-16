use betterlite_db::key_attribution::metadata::KeyMetadata;
use std::collections::BTreeMap;

#[must_use]
pub fn metadata_with_recovered_owner(
    metadata: &BTreeMap<String, KeyMetadata>,
    key: &str,
    owner: &str,
) -> KeyMetadata {
    let mut recovered = metadata.get(key).cloned().unwrap_or_default();
    recovered.user_id = Some(owner.to_owned());
    recovered
}
