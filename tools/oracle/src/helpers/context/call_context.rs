use betterlite_context::call_context::{CallContext, Phase};
use serde::Deserialize;
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    PhaseNesting,
    PinnedBillingTime,
    ChildInheritance,
    #[serde(rename = "dispatch_views_preserve_absence_and_identity")]
    DispatchViews,
    #[serde(rename = "provider_items_preserve_explicit_none")]
    ProviderItems,
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> crate::helpers::HelperResult {
        let id = Uuid::from_u128(1);
        let now = UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .unwrap_or(UNIX_EPOCH);
        let context = CallContext::with_id(id, now);
        let value = match input {
            Input::PhaseNesting => {
                let child = context.post_response();
                json!({"phase":child.view().phase == Phase::PostResponse,"parent_phase":context.view().phase == Phase::PostResponse})
            }
            Input::PinnedBillingTime => {
                json!({"billing_time_unix":context.view().billing_time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()})
            }
            Input::ChildInheritance => {
                let child = context.child(Uuid::from_u128(2));
                json!({"parent_id":child.view().parent_id.map(|parent| parent.to_string()),"internal":child.view().billing == betterlite_context::call_context::BillingPolicy::Suppress,"billing_time_unix":child.view().billing_time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()})
            }
            Input::DispatchViews => {
                json!({"metadata":context.view().metadata.is_none(),"http":context.view().http_client.is_none(),"azure":context.view().azure_client.is_none(),"openai":context.view().openai_client.is_none()})
            }
            Input::ProviderItems => {
                let values: std::collections::BTreeMap<String, Option<&dyn std::fmt::Debug>> =
                    std::collections::BTreeMap::from([("none".to_owned(), None)]);
                json!({"count":values.len(),"is_none":values.iter().next().is_some_and(|(_, value)| value.is_none())})
            }
        };
        Ok(value)
    }
}
