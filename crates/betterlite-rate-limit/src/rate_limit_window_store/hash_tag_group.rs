use super::WindowStore;
use super::cluster_detect::is_redis_cluster;
use super::slot_compute::keyslot_for_redis_cluster;

pub const REDIS_NODE_HASHTAG_NAME: &str = "all_keys";

#[must_use]
pub fn group_keys_by_hash_tag(store: &WindowStore, keys: &[String]) -> Vec<(String, Vec<String>)> {
    if is_redis_cluster(store.cache.as_ref()) {
        return cluster_groups(keys);
    }
    vec![(REDIS_NODE_HASHTAG_NAME.to_owned(), keys.to_vec())]
}

fn cluster_groups(keys: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for key in keys {
        let slot = keyslot_for_redis_cluster(key);
        let tag = format!("slot_{slot}");
        let position = groups
            .iter()
            .position(|(existing, _members)| existing == &tag);
        match position {
            Some(index) => {
                if let Some((_existing, members)) = groups.get_mut(index) {
                    members.push(key.clone());
                }
            }
            None => groups.push((tag, vec![key.clone()])),
        }
    }
    groups
}

mod tests;
