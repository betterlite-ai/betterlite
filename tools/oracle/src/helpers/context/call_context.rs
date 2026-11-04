use betterlite_context::call_context::{BillingPolicy, CallContext, CallInputs, Phase};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use std::ptr::eq;
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
        let context = CallContext::start(CallInputs {
            id: || Uuid::from_u128(1),
            now: || {
                UNIX_EPOCH
                    .checked_add(Duration::from_secs(1_700_000_000))
                    .unwrap_or(UNIX_EPOCH)
            },
            deadline: None,
            privacy: false,
        });
        let value = match input {
            Input::PhaseNesting => {
                let child = context.post_response().post_response();
                json!({"phase":child.view().state.phase == Phase::PostResponse,"parent_phase":context.view().state.phase == Phase::PostResponse})
            }
            Input::PinnedBillingTime => {
                json!({"billing_time_unix":context.view().state.billing_time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()})
            }
            Input::ChildInheritance => {
                let child = context.child(Uuid::from_u128(2));
                json!({"parent_id":child.view().state.parent_id.map(|parent| parent.to_string()),"internal":child.view().state.billing == BillingPolicy::Suppress,"parent_internal":context.view().state.billing == BillingPolicy::Suppress,"billing_time_unix":child.view().state.billing_time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()})
            }
            Input::DispatchViews => dispatch_views(context),
            Input::ProviderItems => provider_items(context),
        };
        Ok(value)
    }
}

fn dispatch_views(context: CallContext<'_>) -> serde_json::Value {
    let absent = context.view().metadata.is_none() && context.view().client.is_none();
    let metadata = [("marker", 7_i32)];
    let client = String::from("client");
    let attached = context.with_dispatch(Some(&metadata), Some(&client));
    let child = attached.child(Uuid::from_u128(2));
    let view = child.view();
    let same_client = view
        .client
        .is_some_and(|stored| eq(stored, &raw const client));
    json!({
        "absent": absent,
        "metadata": view.metadata.is_some_and(|stored| eq(stored, &raw const metadata)),
        "http": same_client, "azure": same_client, "openai": same_client
    })
}

fn provider_items(context: CallContext<'_>) -> serde_json::Value {
    let values = IndexMap::from([("z", None), ("a", Some(7_i32))]);
    let attached = context.with_dispatch(Some(&values), None::<&()>);
    let stored = attached.view().metadata;
    json!({
        "items": stored.map(|mapping| mapping.iter().collect::<Vec<_>>()),
        "identity": stored.is_some_and(|items| eq(items, &raw const values))
    })
}
