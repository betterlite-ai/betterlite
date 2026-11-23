use std::collections::BTreeMap;

// The optional pre-call check's name in model_group_affinity_config: only a group whose list carries it enables the filter.
pub const AFFINITY_CHECK: &str = "encrypted_content_affinity";
// The request metadata key the response post-processor reads to encode encrypted item ids.
pub const ENABLED_FLAG: &str = "encrypted_content_affinity_enabled";

// EncryptedContentAffinityCheck.__init__'s resolved settings, with the constructor's true global default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffinitySettings {
    pub enable_global_affinity: bool,
    pub model_group_affinity_config: BTreeMap<String, Vec<String>>,
}

impl Default for AffinitySettings {
    fn default() -> Self {
        Self {
            enable_global_affinity: true,
            model_group_affinity_config: BTreeMap::new(),
        }
    }
}

impl AffinitySettings {
    // has_model_group_affinity_enabled: any group's list names the check, so an empty config registers nothing.
    #[must_use]
    pub fn has_group_affinity_enabled(&self) -> bool {
        self.model_group_affinity_config
            .values()
            .any(|checks| checks.iter().any(|check| check == AFFINITY_CHECK))
    }

    // _is_enabled_for_model_group: the global switch, or this one group's own list.
    #[must_use]
    pub fn enabled_for_group(&self, model_group: &str) -> bool {
        self.enable_global_affinity
            || self
                .model_group_affinity_config
                .get(model_group)
                .is_some_and(|checks| checks.iter().any(|check| check == AFFINITY_CHECK))
    }
}
