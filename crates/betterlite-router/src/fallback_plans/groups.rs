#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataValues {
    pub pre_routing_selected_model: Option<String>,
    pub model_group: Option<String>,
    pub original_model_group: Option<String>,
    pub disable_fallbacks: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bucket {
    pub present: bool,
    pub values: MetadataValues,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestMetadata {
    pub metadata: Bucket,
    pub litellm_metadata: Bucket,
}

impl RequestMetadata {
    const fn selected(&mut self) -> &mut Bucket {
        if self.litellm_metadata.present {
            &mut self.litellm_metadata
        } else {
            &mut self.metadata
        }
    }

    const fn selected_ref(&self) -> &Bucket {
        if self.litellm_metadata.present {
            &self.litellm_metadata
        } else {
            &self.metadata
        }
    }
}

#[must_use]
pub fn fallback_lookup_groups<'state>(
    state: &'state RequestMetadata,
    model_group: Option<&'state str>,
) -> Vec<&'state str> {
    let bucket = state.selected_ref();
    let ordered = [
        get_pre_routing_selection(state),
        bucket.values.model_group.as_deref(),
        model_group,
        bucket.values.original_model_group.as_deref(),
    ];
    let mut groups: Vec<&str> = Vec::new();
    for group in ordered.into_iter().flatten() {
        if !group.is_empty() && !groups.contains(&group) {
            groups.push(group);
        }
    }
    groups
}

#[must_use]
pub fn get_pre_routing_selection(state: &RequestMetadata) -> Option<&str> {
    for bucket in [&state.metadata, &state.litellm_metadata] {
        let selected = bucket.values.pre_routing_selected_model.as_deref();
        if let Some(found) = selected.filter(|value| !value.is_empty()) {
            return Some(found);
        }
    }
    None
}

pub fn record_pre_routing_selection(state: &mut RequestMetadata, selected_model: &str) {
    let bucket = state.selected();
    if bucket.present {
        bucket.values.pre_routing_selected_model = Some(selected_model.to_owned());
    }
}

// A fallback hop also inherits the previous hop's selection, so every hop drops any value before routing its own attempt
pub fn clear_pre_routing_selection(state: &mut RequestMetadata) {
    for bucket in [&mut state.metadata, &mut state.litellm_metadata] {
        bucket.values.pre_routing_selected_model = None;
    }
}

pub fn carry_pre_routing_selection(live: &RequestMetadata, snapshot: &mut RequestMetadata) {
    clear_pre_routing_selection(snapshot);
    if let Some(selection) = get_pre_routing_selection(live) {
        record_pre_routing_selection(snapshot, selection);
    }
}

#[must_use]
pub fn fallbacks_disabled_for_request(
    state: &RequestMetadata,
    raw_disable_fallbacks: bool,
) -> bool {
    if raw_disable_fallbacks {
        return true;
    }
    [&state.metadata, &state.litellm_metadata]
        .into_iter()
        .any(|bucket| bucket.present && bucket.values.disable_fallbacks)
}

pub const fn record_disable_fallbacks(state: &mut RequestMetadata, disabled: bool) {
    let bucket = state.selected();
    if bucket.present {
        bucket.values.disable_fallbacks = disabled;
    }
}
