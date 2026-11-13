use std::collections::BTreeMap;

use betterlite_policy::attachment::{
    AttachmentRegistry, attachment_sort_key, attachment_specificity,
};
use betterlite_policy::context::PolicyMatchContext;
use betterlite_policy::matcher::policy_applies;
use betterlite_policy::registry::RawPolicy;
use betterlite_policy::values::PolicyAttachment;
use serde::Deserialize;
use serde_json::{Value, json};

use super::support;
use crate::helpers::HelperResult;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Action {
    LoadAttachments {
        attachments: Vec<PolicyAttachment>,
    },
    ParseAttachment {
        attachment: PolicyAttachment,
    },
    Specificity {
        attachments: Vec<PolicyAttachment>,
    },
    SortKey {
        attachments: Vec<PolicyAttachment>,
    },
    AttachedPolicies {
        attachments: Vec<PolicyAttachment>,
        context: PolicyMatchContext,
        policies: Option<BTreeMap<String, RawPolicy>>,
    },
}

pub(crate) fn run(action: &Action) -> HelperResult {
    match action {
        Action::LoadAttachments { attachments } => Ok(loaded(attachments)),
        Action::ParseAttachment { attachment } => Ok(json!({ "attachment": attachment })),
        Action::Specificity { attachments } => Ok(specificity(attachments)),
        Action::SortKey { attachments } => Ok(sort_keys(attachments)),
        Action::AttachedPolicies {
            attachments,
            context,
            policies,
        } => attached(attachments, context, policies.as_ref()),
    }
}

fn loaded(attachments: &[PolicyAttachment]) -> Value {
    let mut registry = AttachmentRegistry::default();
    registry.load_attachments(attachments.to_vec());
    json!({
        "initialized": registry.initialized,
        "attachments": registry.attachments,
        "config_attachments": registry.config_attachments,
    })
}

fn specificity(attachments: &[PolicyAttachment]) -> Value {
    let mut values = Vec::new();
    for attachment in attachments {
        let (highest, dimensions) = attachment_specificity(attachment);
        values.push(json!([highest, dimensions]));
    }
    json!({ "specificity": values })
}

fn sort_keys(attachments: &[PolicyAttachment]) -> Value {
    let mut keys = Vec::new();
    for attachment in attachments {
        keys.push(json!(attachment_sort_key(attachment)));
    }
    json!({ "sort_keys": keys })
}

fn attached(
    attachments: &[PolicyAttachment],
    context: &PolicyMatchContext,
    policy_config: Option<&BTreeMap<String, RawPolicy>>,
) -> HelperResult {
    let mut registry = AttachmentRegistry::default();
    registry.load_attachments(attachments.to_vec());
    let Some(policies) = policy_config else {
        let names = registry.get_attached_policies(context, None);
        let matches = registry.attached_policy_matches(context, None);
        return Ok(json!({ "policies": names, "matches": matches }));
    };
    let policy_registry = support::loaded(policies)?;
    let applies = |name: &str| policy_applies(name, context, &policy_registry.policies);
    let names = registry.get_attached_policies(context, Some(&applies));
    let matches = registry.attached_policy_matches(context, Some(&applies));
    Ok(json!({ "policies": names, "matches": matches }))
}
