mod tests;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::context::PolicyMatchContext;
use crate::scope::{matches_pattern, scope_matches};
use crate::values::PolicyAttachment;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyAttachmentMatch {
    pub policy_name: String,
    pub matched_via: String,
}

#[derive(Debug, Clone, Default)]
pub struct AttachmentRegistry {
    pub attachments: Vec<PolicyAttachment>,
    pub config_attachments: Vec<PolicyAttachment>,
    pub initialized: bool,
}

#[must_use]
pub fn attachment_specificity(attachment: &PolicyAttachment) -> (u8, u8) {
    if attachment.is_global() {
        return (0, 0);
    }
    let mut highest: u8 = 0;
    let mut dimensions: u8 = 0;
    let ranked = [
        (&attachment.teams, 1),
        (&attachment.keys, 2),
        (&attachment.tags, 3),
        (&attachment.models, 4),
    ];
    for (values, rank) in ranked {
        if values.as_ref().is_some_and(|list| !list.is_empty()) {
            highest = highest.max(rank);
            dimensions = dimensions.saturating_add(1);
        }
    }
    (highest, dimensions)
}

#[must_use]
pub fn attachment_sort_key(attachment: &PolicyAttachment) -> (u8, i32, u8, u8) {
    let (highest, dimensions) = attachment_specificity(attachment);
    attachment
        .priority
        .map_or((1, 0, highest, dimensions), |priority| {
            (0, priority, highest, dimensions)
        })
}

fn describe_scope_match(attachment: &PolicyAttachment, context: &PolicyMatchContext) -> String {
    if attachment.is_global() {
        return "scope:*".to_owned();
    }
    let mut reasons = Vec::new();
    if let Some(tags) = attachment.tags.as_deref().filter(|list| !list.is_empty())
        && let Some(context_tags) = context.tags.as_ref().filter(|list| !list.is_empty())
        && let Some(matched) = context_tags
            .iter()
            .find(|tag| matches_pattern(Some(tag.as_str()), tags))
    {
        reasons.push(format!("tag:{matched}"));
    }
    if attachment
        .teams
        .as_ref()
        .is_some_and(|teams| !teams.is_empty())
        && let Some(team_alias) = context.team_alias.as_deref()
    {
        reasons.push(format!("team:{team_alias}"));
    }
    if attachment
        .keys
        .as_ref()
        .is_some_and(|keys| !keys.is_empty())
        && let Some(key_alias) = context.key_alias.as_deref()
    {
        reasons.push(format!("key:{key_alias}"));
    }
    if attachment
        .models
        .as_ref()
        .is_some_and(|models| !models.is_empty())
        && let Some(model) = context.model.as_deref()
    {
        reasons.push(format!("model:{model}"));
    }
    if reasons.is_empty() {
        return "scope:default".to_owned();
    }
    reasons.join("+")
}

fn describe_match_reason(attachment: &PolicyAttachment, context: &PolicyMatchContext) -> String {
    let reason = describe_scope_match(attachment, context);
    if attachment.default {
        return format!("default:{reason}");
    }
    reason
}

impl AttachmentRegistry {
    pub fn load_attachments(&mut self, attachments: Vec<PolicyAttachment>) {
        for attachment in &attachments {
            tracing::debug!("Loaded attachment for policy: {}", attachment.policy);
        }
        self.attachments.clone_from(&attachments);
        self.config_attachments = attachments;
        self.initialized = true;
        tracing::info!("Loaded {} policy attachments", self.attachments.len());
    }

    pub fn add_attachment(&mut self, attachment: PolicyAttachment) {
        tracing::debug!("Added attachment for policy: {}", attachment.policy);
        self.attachments.push(attachment);
        self.initialized = true;
    }

    #[must_use]
    pub fn attached_policy_matches(
        &self,
        context: &PolicyMatchContext,
        policy_applies: Option<&dyn Fn(&str) -> bool>,
    ) -> Vec<PolicyAttachmentMatch> {
        let in_scope: Vec<&PolicyAttachment> = self
            .attachments
            .iter()
            .filter(|attachment| scope_matches(&attachment.to_policy_scope(), context))
            .filter(|attachment| policy_applies.is_none_or(|applies| applies(&attachment.policy)))
            .collect();
        let non_default: Vec<&PolicyAttachment> = in_scope
            .iter()
            .copied()
            .filter(|attachment| !attachment.default)
            .collect();
        let mut candidates: Vec<&PolicyAttachment> = if non_default.is_empty() {
            in_scope
                .iter()
                .copied()
                .filter(|attachment| attachment.default)
                .collect()
        } else {
            non_default
        };
        candidates.sort_by_key(|attachment| attachment_sort_key(attachment));
        let mut seen = BTreeSet::new();
        let mut matches = Vec::new();
        for attachment in candidates {
            if !seen.insert(attachment.policy.clone()) {
                continue;
            }
            matches.push(PolicyAttachmentMatch {
                policy_name: attachment.policy.clone(),
                matched_via: describe_match_reason(attachment, context),
            });
        }
        matches
    }

    #[must_use]
    pub fn get_attached_policies(
        &self,
        context: &PolicyMatchContext,
        policy_applies: Option<&dyn Fn(&str) -> bool>,
    ) -> Vec<String> {
        let mut names = Vec::new();
        for attachment_match in self.attached_policy_matches(context, policy_applies) {
            names.push(attachment_match.policy_name);
        }
        names
    }
}
