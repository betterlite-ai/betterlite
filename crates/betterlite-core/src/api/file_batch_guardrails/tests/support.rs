#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use crate::api::file_batch_guardrails::hook::{
    GuardrailsOnlyPreCall, PreCallInput, RaisedGuardrail,
};
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::record_json::{RecordJson, RecordObject, load_record};
use crate::routes::CallType;
use crate::routes::catalog::{CATALOG, RouteCatalog};

pub(super) fn object(text: &str) -> RecordObject {
    load_record(text.as_bytes()).unwrap()
}

pub(super) fn member<'value>(members: &'value RecordObject, key: &str) -> &'value RecordJson {
    members.get(&RecordText::from(key)).unwrap()
}

pub(super) fn routes() -> RouteCatalog {
    RouteCatalog::parse(CATALOG).unwrap()
}

pub(super) fn principal() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap()
}

// A GuardrailRaisedException says whether it blocked; anything else only carries a status and maybe a cause
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Raised {
    Verdict { blocked: bool, caused: bool },
    Status { intervention: bool, caused: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FakeFailure {
    pub(super) raised: Raised,
    pub(super) name: Option<String>,
    pub(super) detail_name: Option<String>,
}

pub(super) fn named(raised: Raised) -> FakeFailure {
    FakeFailure {
        raised,
        name: Some("g".to_owned()),
        detail_name: None,
    }
}

impl RaisedGuardrail for FakeFailure {
    fn blocked_content(&self) -> Option<bool> {
        match self.raised {
            Raised::Verdict { blocked, .. } => Some(blocked),
            Raised::Status { .. } => None,
        }
    }

    fn has_cause(&self) -> bool {
        match self.raised {
            Raised::Verdict { caused, .. } | Raised::Status { caused, .. } => caused,
        }
    }

    fn is_intervention(&self) -> bool {
        match self.raised {
            Raised::Verdict { .. } => true,
            Raised::Status { intervention, .. } => intervention,
        }
    }

    fn guardrail_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn detail_guardrail_name(&self) -> Option<&str> {
        self.detail_name.as_deref()
    }
}

pub(super) enum Answer {
    Rewrite(fn(RecordObject) -> RecordObject),
    // Sets one key of the scanned body to this JSON text
    Insert(&'static str, &'static str),
    Raise(FakeFailure),
}

#[derive(Debug)]
pub(super) struct SeenCall {
    pub(super) data: RecordObject,
    pub(super) call_type: CallType,
    pub(super) guardrails_only: bool,
}

pub(super) struct FakeHook {
    pub(super) answer: Answer,
    pub(super) seen: Mutex<Vec<SeenCall>>,
}

#[async_trait]
impl GuardrailsOnlyPreCall for FakeHook {
    type Failure = FakeFailure;

    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, FakeFailure> {
        self.seen.lock().unwrap().push(SeenCall {
            data: input.data.clone(),
            call_type: input.call_type,
            guardrails_only: input.guardrails_only,
        });
        match &self.answer {
            Answer::Rewrite(rewrite) => Ok(rewrite(input.data)),
            Answer::Insert(key, value) => {
                let mut data = input.data;
                let wrapped = object(&format!(r#"{{"value":{value}}}"#));
                data.insert(RecordText::from(*key), member(&wrapped, "value").clone());
                Ok(data)
            }
            Answer::Raise(failure) => Err(failure.clone()),
        }
    }
}
