#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_registry::callback::{
    CallbackScoping, CallbackSpec, GuardrailCallback,
};
use betterlite_guardrails::guardrail_registry::integrations::CompressionKind;

use crate::registry_read_through::RegistryReadThrough;
use crate::registry_read_through::Resync;
use crate::registry_read_through::ResyncError;
use crate::registry_read_through::lookup::{
    AgentLookup, GuardrailLookup, get_agent_with_read_through, get_guardrail_with_read_through,
};

struct FakeAgentRegistry {
    agents: Arc<Mutex<Vec<AgentResponse>>>,
    lookups: Arc<AtomicUsize>,
}

impl AgentLookup for FakeAgentRegistry {
    fn by_id_or_name(&self, key: &str) -> Option<AgentResponse> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.agents
            .lock()
            .unwrap()
            .iter()
            .find(|agent| agent.agent_id == key || agent.agent_name == key)
            .cloned()
    }
}

struct AgentLoadResync {
    agents: Arc<Mutex<Vec<AgentResponse>>>,
    agent: AgentResponse,
    found: bool,
    calls: Arc<AtomicUsize>,
}

impl Resync for AgentLoadResync {
    fn resync(&self, _key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.found {
            self.agents.lock().unwrap().push(self.agent.clone());
        }
        ready(Ok(self.found))
    }
}

fn agent(agent_id: &str, agent_name: &str) -> AgentResponse {
    let text = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_name}","agent_card_params":{{}}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn an_agent_hit_is_returned_without_a_read_through() {
    let agents = Arc::new(Mutex::new(vec![agent("agent-1", "support")]));
    let lookups = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = FakeAgentRegistry {
        agents: Arc::clone(&agents),
        lookups: Arc::clone(&lookups),
    };
    let read_through = RegistryReadThrough::new(AgentLoadResync {
        agents: Arc::clone(&agents),
        agent: agent("agent-2", "new-support"),
        found: true,
        calls: Arc::clone(&calls),
    });
    let found = get_agent_with_read_through(&registry, &read_through, "support").await;
    assert_eq!(
        found.map(|agent| agent.agent_id),
        Some("agent-1".to_owned())
    );
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_agent_is_looked_up_again_after_the_read_through_loads_it() {
    let agents = Arc::new(Mutex::new(Vec::new()));
    let lookups = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = FakeAgentRegistry {
        agents: Arc::clone(&agents),
        lookups: Arc::clone(&lookups),
    };
    let read_through = RegistryReadThrough::new(AgentLoadResync {
        agents: Arc::clone(&agents),
        agent: agent("agent-2", "new-support"),
        found: true,
        calls: Arc::clone(&calls),
    });
    let found = get_agent_with_read_through(&registry, &read_through, "new-support").await;
    assert_eq!(
        found.map(|agent| agent.agent_id),
        Some("agent-2".to_owned())
    );
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_agent_missing_from_the_database_returns_none() {
    let agents = Arc::new(Mutex::new(Vec::new()));
    let lookups = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = FakeAgentRegistry {
        agents: Arc::clone(&agents),
        lookups: Arc::clone(&lookups),
    };
    let read_through = RegistryReadThrough::new(AgentLoadResync {
        agents: Arc::clone(&agents),
        agent: agent("agent-2", "new-support"),
        found: false,
        calls: Arc::clone(&calls),
    });
    let found = get_agent_with_read_through(&registry, &read_through, "ghost-agent").await;
    assert!(found.is_none());
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct FakeGuardrailRegistry {
    callback: GuardrailCallback,
    present: Arc<AtomicBool>,
    lookups: Arc<AtomicUsize>,
}

impl GuardrailLookup for FakeGuardrailRegistry {
    fn initialized(&self, guardrail_name: &str) -> Option<&GuardrailCallback> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        if !self.present.load(Ordering::SeqCst) {
            return None;
        }
        let named = self.callback.base.settings.guardrail_name.as_deref() == Some(guardrail_name);
        named.then_some(&self.callback)
    }
}

struct GuardrailLoadResync {
    present: Arc<AtomicBool>,
    found: bool,
    calls: Arc<AtomicUsize>,
}

impl Resync for GuardrailLoadResync {
    fn resync(&self, _key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.found {
            self.present.store(true, Ordering::SeqCst);
        }
        ready(Ok(self.found))
    }
}

fn callback(guardrail_name: &str) -> GuardrailCallback {
    GuardrailCallback {
        base: GuardrailBase {
            settings: GuardrailSettings {
                guardrail_name: Some(guardrail_name.to_owned()),
                supported_event_hooks: None,
                event_hook: None,
                default_on: false,
                mask_request_content: false,
                mask_response_content: false,
                violation_message_template: None,
                end_session_after_n_fails: None,
                on_violation: None,
                realtime_violation_message: None,
                on_sensitive_data: None,
                sensitive_data_route_to_model: None,
                sticky_session_routing: true,
                run_in_parallel: false,
                scan_raw_request: false,
                only_scan_new_messages: false,
                timeout: None,
                switches: GuardrailSwitches::default(),
            },
            mode: None,
        },
        spec: CallbackSpec::Compression(CompressionKind::Headroom),
        scoping: CallbackScoping::default(),
    }
}

#[tokio::test]
async fn an_initialized_guardrail_is_returned_without_a_read_through() {
    let present = Arc::new(AtomicBool::new(true));
    let lookups = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = FakeGuardrailRegistry {
        callback: callback("pii"),
        present: Arc::clone(&present),
        lookups: Arc::clone(&lookups),
    };
    let read_through = RegistryReadThrough::new(GuardrailLoadResync {
        present: Arc::clone(&present),
        found: true,
        calls: Arc::clone(&calls),
    });
    let found = get_guardrail_with_read_through(&registry, &read_through, "pii").await;
    assert!(found.is_some());
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_missing_guardrail_is_looked_up_again_after_the_read_through_loads_it() {
    let present = Arc::new(AtomicBool::new(false));
    let lookups = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = FakeGuardrailRegistry {
        callback: callback("pii"),
        present: Arc::clone(&present),
        lookups: Arc::clone(&lookups),
    };
    let read_through = RegistryReadThrough::new(GuardrailLoadResync {
        present: Arc::clone(&present),
        found: true,
        calls: Arc::clone(&calls),
    });
    let found = get_guardrail_with_read_through(&registry, &read_through, "pii").await;
    assert!(found.is_some());
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_guardrail_missing_from_the_database_returns_none() {
    let present = Arc::new(AtomicBool::new(false));
    let lookups = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = FakeGuardrailRegistry {
        callback: callback("pii"),
        present: Arc::clone(&present),
        lookups: Arc::clone(&lookups),
    };
    let read_through = RegistryReadThrough::new(GuardrailLoadResync {
        present: Arc::clone(&present),
        found: false,
        calls: Arc::clone(&calls),
    });
    let found = get_guardrail_with_read_through(&registry, &read_through, "ghost-guardrail").await;
    assert!(found.is_none());
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
