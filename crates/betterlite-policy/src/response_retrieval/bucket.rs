use std::collections::{BTreeMap, BTreeSet};

use super::{MetadataBucket, PolicyPipelines, RetrievalData};
use crate::resolver::pipelines::pipeline_managed_guardrails;

pub(super) fn attach_new_pipelines(
    data: &mut RetrievalData,
    pipelines: &PolicyPipelines,
    sources: &BTreeMap<String, String>,
) -> Option<PolicyPipelines> {
    let bucket = data.bucket_mut();
    let already_attached = bucket.guardrail_pipelines.clone().unwrap_or_default();
    let attached: BTreeSet<&str> = already_attached
        .iter()
        .map(|(policy_name, _)| policy_name.as_str())
        .collect();
    let added: PolicyPipelines = pipelines
        .iter()
        .filter(|(policy_name, _)| !attached.contains(policy_name.as_str()))
        .cloned()
        .collect();
    if added.is_empty() {
        return None;
    }
    let mut merged = already_attached;
    merged.extend(added.iter().cloned());
    bucket.pipeline_managed_guardrails =
        Some(pipeline_managed_guardrails(&merged).into_iter().collect());
    bucket.guardrail_pipelines = Some(merged);
    add_headers(bucket, &added, sources);
    Some(added)
}

fn add_headers(
    bucket: &mut MetadataBucket,
    added: &PolicyPipelines,
    sources: &BTreeMap<String, String>,
) {
    for (policy_name, _) in added {
        push_once(&mut bucket.applied_policies, policy_name);
    }
    for (_, pipeline) in added {
        for step in &pipeline.steps {
            push_once(&mut bucket.applied_guardrails, &step.guardrail);
        }
    }
    let matched: BTreeMap<String, String> = added
        .iter()
        .filter_map(|(policy_name, _)| {
            sources
                .get(policy_name)
                .map(|reason| (policy_name.clone(), reason.clone()))
        })
        .collect();
    if matched.is_empty() {
        return;
    }
    bucket
        .policy_sources
        .get_or_insert_with(BTreeMap::new)
        .extend(matched);
}

fn push_once(entries: &mut Option<Vec<String>>, value: &str) {
    let list = entries.get_or_insert_with(Vec::new);
    if !list.iter().any(|entry| entry == value) {
        list.push(value.to_owned());
    }
}
