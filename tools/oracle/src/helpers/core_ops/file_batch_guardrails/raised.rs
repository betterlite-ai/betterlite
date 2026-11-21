use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::file_batch_guardrails::hook::{
    RaisedGuardrail, is_content_block, naming_guardrail,
};
use betterlite_guardrails::guardrail_base::failure::{
    GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure,
};
use indexmap::IndexMap;
use serde_json::json;

use betterlite_oracle_helper_api::helper::HelperResult;

use super::{Cause, Item, Raise};

// What Python keeps on the exception object besides B5471's fields: an explicit __cause__ and a guardrail_name attribute
struct Raised<'item> {
    failure: GuardrailFailure,
    item: &'item Item<Raise>,
}

impl RaisedGuardrail for Raised<'_> {
    fn blocked_content(&self) -> Option<bool> {
        let GuardrailFailure::GuardrailRaised {
            blocked_content, ..
        } = &self.failure
        else {
            return None;
        };
        Some(*blocked_content)
    }

    fn has_cause(&self) -> bool {
        self.item.cause == Cause::Explicit
    }

    fn is_intervention(&self) -> bool {
        self.failure.is_intervention()
    }

    // A plain exception carries no guardrail_name attribute at all
    fn guardrail_name(&self) -> Option<&str> {
        let named = self.item.guardrail_name.as_deref();
        named.filter(|_| self.item.kind != Raise::Other)
    }

    fn detail_guardrail_name(&self) -> Option<&str> {
        let GuardrailFailure::HttpObject { detail, .. } = &self.failure else {
            return None;
        };
        let Some(GuardrailValue::Text(name)) = detail.get("guardrail_name") else {
            return None;
        };
        Some(name)
    }
}

pub(super) fn content_block(item: &Item<Raise>) -> HelperResult {
    let raised = Raised {
        failure: failure(item),
        item,
    };
    let blocked = is_content_block(&raised);
    Ok(json!({"blocked": blocked, "guardrail": naming_guardrail(&raised)}))
}

// The exception LiteLLM's classifier sees, built with the defaults its constructors use
fn failure(raised: &Item<Raise>) -> GuardrailFailure {
    let guardrail_name = raised.guardrail_name.clone();
    match raised.kind {
        Raise::GuardrailRaised => GuardrailFailure::GuardrailRaised {
            guardrail_name,
            message: String::new(),
            wrap_default_message: true,
            status_code: 400,
            blocked_content: raised.blocked_content,
        },
        Raise::BlockedPii => GuardrailFailure::BlockedPii {
            entity_type: "EMAIL_ADDRESS".to_owned(),
            guardrail_name,
            status_code: 400,
        },
        Raise::ModifyResponse => {
            GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
                message: "blocked".to_owned(),
                model: "m".to_owned(),
                request_data: BTreeMap::new(),
                guardrail_name,
                detection_info: BTreeMap::new(),
                original_response: None,
            }))
        }
        Raise::SensitiveDataRoute => {
            GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
                route_to_model: "local".to_owned(),
                session_id: "s".to_owned(),
                guardrail_name,
                detection_info: BTreeMap::new(),
                message: None,
                sticky_session_routing: false,
            }))
        }
        Raise::Http => http_failure(raised.status, raised.detail_guardrail_name.as_deref()),
        Raise::Other => GuardrailFailure::Other("not a guardrail failure".to_owned()),
    }
}

// An HTTPException's detail is a dict when the guardrail recorded its name there
fn http_failure(status: u16, detail_name: Option<&str>) -> GuardrailFailure {
    let Some(name) = detail_name else {
        let detail = "blocked".to_owned();
        return GuardrailFailure::Http { status, detail };
    };
    let error = GuardrailValue::Text("blocked".to_owned());
    let named = GuardrailValue::Text(name.to_owned());
    let detail = IndexMap::from([
        ("error".to_owned(), error),
        ("guardrail_name".to_owned(), named),
    ]);
    GuardrailFailure::HttpObject { status, detail }
}
