#![cfg(test)]

use rstest::rstest;

use super::{AttachmentRegistry, attachment_sort_key, attachment_specificity};
use crate::context::PolicyMatchContext;
use crate::values::PolicyAttachment;

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn attachment(policy: &str) -> PolicyAttachment {
    PolicyAttachment {
        policy: policy.to_owned(),
        scope: None,
        teams: None,
        keys: None,
        models: None,
        tags: None,
        priority: None,
        default: false,
    }
}

fn global() -> PolicyAttachment {
    let mut attachment = attachment("global");
    attachment.scope = Some("*".to_owned());
    attachment
}

fn teams() -> PolicyAttachment {
    let mut attachment = attachment("teams");
    attachment.teams = Some(names(&["team-a"]));
    attachment
}

fn keys() -> PolicyAttachment {
    let mut attachment = attachment("keys");
    attachment.keys = Some(names(&["key-a"]));
    attachment
}

fn tags() -> PolicyAttachment {
    let mut attachment = attachment("tags");
    attachment.tags = Some(names(&["prod"]));
    attachment
}

fn models() -> PolicyAttachment {
    let mut attachment = attachment("models");
    attachment.models = Some(names(&["gpt-4"]));
    attachment
}

fn team_and_model() -> PolicyAttachment {
    let mut attachment = attachment("team-model");
    attachment.teams = Some(names(&["team-a"]));
    attachment.models = Some(names(&["gpt-4"]));
    attachment
}

fn empty_lists() -> PolicyAttachment {
    let mut attachment = attachment("empty");
    attachment.teams = Some(Vec::new());
    attachment.keys = Some(Vec::new());
    attachment.models = Some(Vec::new());
    attachment
}

fn context(
    team_alias: Option<&str>,
    key_alias: Option<&str>,
    model: Option<&str>,
    tags: Option<&[&str]>,
) -> PolicyMatchContext {
    PolicyMatchContext {
        team_alias: team_alias.map(str::to_owned),
        key_alias: key_alias.map(str::to_owned),
        model: model.map(str::to_owned),
        tags: tags.map(names),
    }
}

fn loaded(attachments: Vec<PolicyAttachment>) -> AttachmentRegistry {
    let mut registry = AttachmentRegistry::default();
    registry.load_attachments(attachments);
    registry
}

#[rstest]
#[case::global(global(), (0, 0))]
#[case::teams(teams(), (1, 1))]
#[case::keys(keys(), (2, 1))]
#[case::tags(tags(), (3, 1))]
#[case::models(models(), (4, 1))]
#[case::team_and_model(team_and_model(), (4, 2))]
#[case::empty_lists(empty_lists(), (0, 0))]
fn specificity_ranks_each_dimension(
    #[case] attachment: PolicyAttachment,
    #[case] expected: (u8, u8),
) {
    assert_eq!(attachment_specificity(&attachment), expected);
}

#[test]
fn an_explicit_priority_precedes_the_scope_rank() {
    let mut prioritized = teams();
    prioritized.priority = Some(5_i32);
    assert_eq!(attachment_sort_key(&prioritized), (0, 5_i32, 1, 1));
    assert_eq!(attachment_sort_key(&models()), (1, 0_i32, 4, 1));
    let mut negative = global();
    negative.priority = Some(-1_i32);
    assert_eq!(attachment_sort_key(&negative), (0, -1_i32, 0, 0));
}

#[test]
fn unknown_attachment_keys_are_rejected() {
    let error =
        serde_json::from_str::<PolicyAttachment>(r#"{"policy": "x", "team": ["a"]}"#).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn an_attachment_needs_a_policy_name() {
    let error = serde_json::from_str::<PolicyAttachment>(r#"{"teams": ["a"]}"#).unwrap_err();
    assert!(error.to_string().contains("missing field `policy`"));
}

#[test]
fn load_attachments_records_the_config_snapshot() {
    let registry = loaded(vec![global(), teams()]);
    assert!(registry.initialized);
    assert_eq!(registry.attachments, registry.config_attachments);
    assert_eq!(registry.attachments.len(), 2);
}

#[test]
fn add_attachment_appends_and_initializes() {
    let mut registry = AttachmentRegistry::default();
    registry.add_attachment(global());
    assert!(registry.initialized);
    assert_eq!(registry.attachments.len(), 1);
    assert!(registry.config_attachments.is_empty());
}

#[test]
fn a_global_attachment_matches_any_context() {
    let registry = loaded(vec![global()]);
    let empty = context(None, None, None, None);
    assert_eq!(registry.get_attached_policies(&empty, None), ["global"]);
    let full = context(Some("team"), Some("key"), Some("model"), Some(&["prod"]));
    assert_eq!(registry.get_attached_policies(&full, None), ["global"]);
}

#[test]
fn attachments_filter_by_team_key_model_and_tags() {
    let mut team = attachment("team");
    team.teams = Some(names(&["team-a"]));
    let registry = loaded(vec![team]);
    assert_eq!(
        registry.get_attached_policies(&context(Some("team-a"), None, None, None), None),
        ["team"]
    );
    assert!(
        registry
            .get_attached_policies(&context(Some("team-b"), None, None, None), None)
            .is_empty()
    );
    let mut tagged = attachment("tagged");
    tagged.tags = Some(names(&["prod-*"]));
    let tagged_registry = loaded(vec![tagged]);
    assert_eq!(
        tagged_registry.get_attached_policies(&context(None, None, None, Some(&["prod-eu"])), None),
        ["tagged"]
    );
    assert!(
        tagged_registry
            .get_attached_policies(&context(None, None, None, None), None)
            .is_empty()
    );
}

#[test]
fn matching_is_ordered_from_the_broadest_to_the_narrowest_scope() {
    let registry = loaded(vec![models(), teams(), global(), keys(), tags()]);
    let context = context(
        Some("team-a"),
        Some("key-a"),
        Some("gpt-4"),
        Some(&["prod"]),
    );
    assert_eq!(
        registry.get_attached_policies(&context, None),
        ["global", "teams", "keys", "tags", "models"]
    );
}

#[test]
fn a_priority_orders_attachments_before_scope_specificity() {
    let mut unprioritized = attachment("unprioritized-tag");
    unprioritized.tags = Some(names(&["prod"]));
    let mut prioritized = attachment("prioritized-tag");
    prioritized.tags = Some(names(&["prod"]));
    prioritized.priority = Some(5_i32);
    let mut model = attachment("prioritized-model");
    model.models = Some(names(&["gpt-4"]));
    model.priority = Some(0_i32);
    let registry = loaded(vec![unprioritized, prioritized, model]);
    assert_eq!(
        registry.get_attached_policies(&context(None, None, Some("gpt-4"), Some(&["prod"])), None),
        ["prioritized-model", "prioritized-tag", "unprioritized-tag"]
    );
}

#[test]
fn equal_priorities_fall_back_to_the_scope_rank() {
    let mut model = models();
    model.policy = "model-policy".to_owned();
    model.priority = Some(1_i32);
    let mut tag = tags();
    tag.policy = "tag-policy".to_owned();
    tag.priority = Some(1_i32);
    let mut global = global();
    global.policy = "global-policy".to_owned();
    global.priority = Some(1_i32);
    let registry = loaded(vec![model, tag, global]);
    assert_eq!(
        registry.get_attached_policies(&context(None, None, Some("gpt-4"), Some(&["prod"])), None),
        ["global-policy", "tag-policy", "model-policy"]
    );
}

#[test]
fn the_match_reason_names_the_matched_dimension() {
    let registry = loaded(vec![global(), teams(), models()]);
    let matches =
        registry.attached_policy_matches(&context(Some("team-a"), None, Some("gpt-4"), None), None);
    assert_eq!(
        matches,
        [
            super::PolicyAttachmentMatch {
                policy_name: "global".to_owned(),
                matched_via: "scope:*".to_owned(),
            },
            super::PolicyAttachmentMatch {
                policy_name: "teams".to_owned(),
                matched_via: "team:team-a".to_owned(),
            },
            super::PolicyAttachmentMatch {
                policy_name: "models".to_owned(),
                matched_via: "model:gpt-4".to_owned(),
            },
        ]
    );
}

#[test]
fn a_combined_scope_reason_joins_its_dimensions() {
    let mut attachment = attachment("combined");
    attachment.teams = Some(names(&["team-a"]));
    attachment.tags = Some(names(&["prod*"]));
    attachment.models = Some(names(&["gpt-4"]));
    let registry = loaded(vec![attachment]);
    let matches = registry.attached_policy_matches(
        &context(
            Some("team-a"),
            None,
            Some("gpt-4"),
            Some(&["prod-eu", "other"]),
        ),
        None,
    );
    assert_eq!(
        matches[0].matched_via,
        "tag:prod-eu+team:team-a+model:gpt-4"
    );
}

#[test]
fn a_default_reason_carries_the_prefix() {
    let mut attachment = global();
    attachment.policy = "fallback".to_owned();
    attachment.default = true;
    let registry = loaded(vec![attachment]);
    let matches = registry.attached_policy_matches(&context(None, None, None, None), None);
    assert_eq!(matches[0].matched_via, "default:scope:*");
}

#[test]
fn duplicate_policies_keep_the_broadest_match() {
    let mut narrow = attachment("shared");
    narrow.models = Some(names(&["gpt-4"]));
    let mut broad = attachment("shared");
    broad.scope = Some("*".to_owned());
    let mut other = attachment("other");
    other.models = Some(names(&["gpt-4"]));
    let registry = loaded(vec![narrow, other, broad]);
    assert_eq!(
        registry.get_attached_policies(&context(None, None, Some("gpt-4"), None), None),
        ["shared", "other"]
    );
    assert_eq!(
        registry.attached_policy_matches(&context(None, None, Some("gpt-4"), None), None)[0]
            .matched_via,
        "scope:*"
    );
}

#[test]
fn duplicate_policies_prefer_a_single_dimension_over_a_combined_one() {
    let mut combined = attachment("shared");
    combined.teams = Some(names(&["team-a"]));
    combined.models = Some(names(&["gpt-4"]));
    let mut model = attachment("shared");
    model.models = Some(names(&["gpt-4"]));
    let registry = loaded(vec![combined, model]);
    let matches =
        registry.attached_policy_matches(&context(Some("team-a"), None, Some("gpt-4"), None), None);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].matched_via, "model:gpt-4");
}

#[test]
fn a_non_default_match_suppresses_the_defaults() {
    let mut fallback = global();
    fallback.policy = "fallback".to_owned();
    fallback.default = true;
    let mut opt_in = attachment("opt-in");
    opt_in.tags = Some(names(&["opt-in"]));
    let registry = loaded(vec![fallback, opt_in]);
    assert_eq!(
        registry.get_attached_policies(&context(None, None, None, Some(&["opt-in"])), None),
        ["opt-in"]
    );
    assert_eq!(
        registry.get_attached_policies(&context(None, None, None, None), None),
        ["fallback"]
    );
}

#[test]
fn every_matching_default_applies_when_nothing_else_matches() {
    let mut global = global();
    global.policy = "default-a".to_owned();
    global.default = true;
    let mut team = attachment("default-b");
    team.teams = Some(names(&["team-a"]));
    team.default = true;
    let mut opt_in = attachment("opt-in");
    opt_in.tags = Some(names(&["opt-in"]));
    let registry = loaded(vec![global, team, opt_in]);
    assert_eq!(
        registry.get_attached_policies(&context(Some("team-a"), None, None, None), None),
        ["default-a", "default-b"]
    );
}

#[test]
fn the_predicate_drops_inapplicable_policies_before_defaults() {
    let mut fallback = global();
    fallback.policy = "fallback".to_owned();
    fallback.default = true;
    let mut opt_in = attachment("opt-in");
    opt_in.tags = Some(names(&["opt-in"]));
    let registry = loaded(vec![fallback, opt_in]);
    let applies = |policy: &str| policy != "opt-in";
    assert_eq!(
        registry.get_attached_policies(
            &context(None, None, None, Some(&["opt-in"])),
            Some(&applies)
        ),
        ["fallback"]
    );
}

#[test]
fn an_empty_registry_matches_nothing() {
    let registry = loaded(Vec::new());
    assert!(
        registry
            .get_attached_policies(
                &context(Some("team"), Some("key"), Some("model"), None),
                None
            )
            .is_empty()
    );
}
