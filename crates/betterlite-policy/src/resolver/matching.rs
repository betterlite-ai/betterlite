use crate::attachment::AttachmentRegistry;
use crate::context::PolicyMatchContext;
use crate::matcher::get_matching_policies;
use crate::registry::PolicyRegistry;
use crate::values::Policy;

pub(super) fn matching_policies<'registry>(
    context: &PolicyMatchContext,
    registry: &'registry PolicyRegistry,
    attachments: &AttachmentRegistry,
    policy_names: Option<&[String]>,
) -> Vec<(&'registry String, &'registry Policy)> {
    if !registry.initialized {
        return Vec::new();
    }
    let names = policy_names.map_or_else(
        || get_matching_policies(context, attachments, registry),
        <[String]>::to_vec,
    );
    let matching: Vec<_> = names
        .iter()
        .filter_map(|name| registry.policies.get_key_value(name))
        .collect();
    if matching.is_empty() {
        tracing::debug!(
            team_alias = ?context.team_alias,
            key_alias = ?context.key_alias,
            model = ?context.model,
            "no policies match the context"
        );
    }
    matching
}
