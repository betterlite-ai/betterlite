use betterlite_context::call_context::{CallContext, Phase};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fmt::Debug;
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
            Input::DispatchViews => dispatch_views(context),
            Input::ProviderItems => provider_items(context),
        };
        Ok(value)
    }
}

fn dispatch_views(context: CallContext<'_>) -> serde_json::Value {
    let marker: &dyn Debug = &"metadata-value";
    let metadata: BTreeMap<String, Option<&dyn Debug>> =
        BTreeMap::from([("marker".to_owned(), Some(marker))]);
    let http: &dyn Debug = &"http-client";
    let azure: &dyn Debug = &"azure-client";
    let openai: &dyn Debug = &"openai-client";
    let attached = context
        .with_metadata(&metadata)
        .with_http_client(http)
        .with_azure_client(azure)
        .with_openai_client(openai);
    let view = attached.view();
    json!({
        "metadata": view.metadata.is_some_and(|stored| eq(stored, &raw const metadata)),
        "http": view.http_client.is_some_and(|stored| eq(stored, http)),
        "azure": view.azure_client.is_some_and(|stored| eq(stored, azure)),
        "openai": view.openai_client.is_some_and(|stored| eq(stored, openai))
    })
}

fn provider_items(context: CallContext<'_>) -> serde_json::Value {
    let values: BTreeMap<String, Option<&dyn Debug>> = BTreeMap::from([("none".to_owned(), None)]);
    let attached = context.with_metadata(&values);
    let stored = attached.view().metadata;
    json!({
        "count": stored.map_or(0, BTreeMap::len),
        "is_none": stored.is_some_and(|items| {
            items.contains_key("none") && items.get("none").is_some_and(Option::is_none)
        })
    })
}
