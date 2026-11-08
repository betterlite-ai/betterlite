use std::sync::{Arc, Mutex, PoisonError};

use super::traits::{HookFlag, ProxyCallback};

pub type CallbackList = Vec<Arc<dyn ProxyCallback>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IteratorKind {
    Override,
    ApplyGuardrail,
}

#[derive(Clone)]
pub struct IteratorOverride {
    pub callback: Arc<dyn ProxyCallback>,
    pub kind: IteratorKind,
}

#[derive(Clone, Default)]
pub struct CallbackCapabilities {
    pub has_post_call_response_headers: HookFlag,
    pub has_iterator_override: HookFlag,
    pub has_streaming_chunk_override: HookFlag,
    pub has_guardrail: HookFlag,
    pub has_pre_call_override: HookFlag,
    pub has_content_enforcer: HookFlag,
    pub has_moderation_override: HookFlag,
    pub iterator_overrides: Vec<IteratorOverride>,
    pub resolved_callbacks: CallbackList,
    pub listed_models_filters: CallbackList,
}

impl CallbackCapabilities {
    #[must_use]
    pub fn from_callbacks(callbacks: &[Arc<dyn ProxyCallback>]) -> Self {
        let mut walk = CapabilityWalk::default();
        for callback in callbacks {
            walk.visit(callback);
        }
        walk.finish()
    }

    #[must_use]
    pub const fn has_streaming_callbacks(&self) -> bool {
        if self.has_iterator_override {
            true
        } else {
            self.has_streaming_chunk_override || self.has_guardrail
        }
    }

    #[must_use]
    pub const fn has_chunk_hook_overrides(&self) -> bool {
        if self.has_streaming_chunk_override {
            true
        } else {
            self.has_guardrail
        }
    }

    // LiteLLM asks this and has_chunk_hook_overrides as separate questions with the same rule
    #[must_use]
    pub const fn needs_per_chunk_streaming_hook(&self) -> bool {
        if self.has_streaming_chunk_override {
            true
        } else {
            self.has_guardrail
        }
    }

    #[must_use]
    pub fn partition_post_call(&self) -> (CallbackList, CallbackList) {
        let mut guardrails = Vec::new();
        let mut others = Vec::new();
        for callback in &self.resolved_callbacks {
            if callback.guardrail().is_some() {
                guardrails.push(Arc::clone(callback));
            } else {
                others.push(Arc::clone(callback));
            }
        }
        (guardrails, others)
    }
}

#[derive(Default)]
struct CapabilityWalk {
    has_post_call_response_headers: HookFlag,
    has_streaming_chunk_override: HookFlag,
    has_guardrail: HookFlag,
    has_pre_call_override: HookFlag,
    has_content_enforcer: HookFlag,
    has_moderation_override: HookFlag,
    streaming_iterator_override: HookFlag,
    iterator_overrides: Vec<IteratorOverride>,
    resolved_callbacks: CallbackList,
    listed_models_filters: CallbackList,
}

impl CapabilityWalk {
    fn visit(&mut self, callback: &Arc<dyn ProxyCallback>) {
        self.resolved_callbacks.push(Arc::clone(callback));
        if callback.is_base_logger() {
            return;
        }
        let overrides = callback.overrides();
        let is_guardrail = callback.guardrail().is_some();
        self.has_guardrail |= is_guardrail;
        self.has_moderation_override |= overrides.moderation && !is_guardrail;
        if overrides.response_headers {
            self.has_post_call_response_headers = true;
        }
        self.streaming_iterator_override |= overrides.streaming_iterator;
        let kind = if overrides.streaming_iterator {
            Some(IteratorKind::Override)
        } else if overrides.apply_guardrail && !callback.use_native_lifecycle_hooks() {
            Some(IteratorKind::ApplyGuardrail)
        } else {
            None
        };
        if let Some(entry_kind) = kind {
            self.iterator_overrides.push(IteratorOverride {
                callback: Arc::clone(callback),
                kind: entry_kind,
            });
        }
        if overrides.streaming_chunk {
            self.has_streaming_chunk_override = true;
        }
        if overrides.pre_call {
            self.has_pre_call_override = true;
            if callback.enforces_request_content() {
                self.has_content_enforcer = true;
            }
        }
        if overrides.filter_listed_models {
            self.listed_models_filters.push(Arc::clone(callback));
        }
    }

    fn finish(self) -> CallbackCapabilities {
        let has_iterator_override = self.streaming_iterator_override
            || self
                .iterator_overrides
                .iter()
                .any(|entry| entry.kind == IteratorKind::ApplyGuardrail);
        CallbackCapabilities {
            has_post_call_response_headers: self.has_post_call_response_headers,
            has_iterator_override,
            has_streaming_chunk_override: self.has_streaming_chunk_override,
            has_guardrail: self.has_guardrail,
            has_pre_call_override: self.has_pre_call_override,
            has_content_enforcer: self.has_content_enforcer,
            has_moderation_override: self.has_moderation_override,
            iterator_overrides: self.iterator_overrides,
            resolved_callbacks: self.resolved_callbacks,
            listed_models_filters: self.listed_models_filters,
        }
    }
}

#[derive(Default)]
pub struct CallbackCache {
    cached: Mutex<Option<CachedCapabilities>>,
}

struct CachedCapabilities {
    callbacks: CallbackList,
    capabilities: Arc<CallbackCapabilities>,
}

impl CachedCapabilities {
    fn matches(&self, callbacks: &[Arc<dyn ProxyCallback>]) -> bool {
        if self.callbacks.len() != callbacks.len() {
            return false;
        }
        self.callbacks
            .iter()
            .zip(callbacks)
            .all(|(cached, incoming)| Arc::ptr_eq(cached, incoming))
    }
}

impl CallbackCache {
    #[must_use]
    pub fn capabilities(&self, callbacks: &[Arc<dyn ProxyCallback>]) -> Arc<CallbackCapabilities> {
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        let existing = cached.as_ref().filter(|entry| entry.matches(callbacks));
        if let Some(entry) = existing {
            return Arc::clone(&entry.capabilities);
        }
        let capabilities = Arc::new(CallbackCapabilities::from_callbacks(callbacks));
        *cached = Some(CachedCapabilities {
            callbacks: callbacks.to_vec(),
            capabilities: Arc::clone(&capabilities),
        });
        capabilities
    }
}

#[must_use]
pub fn combined_callback_list(
    dynamic: Option<&[Arc<dyn ProxyCallback>]>,
    global: &[Arc<dyn ProxyCallback>],
) -> CallbackList {
    let Some(dynamic_list) = dynamic else {
        return global.to_vec();
    };
    let mut combined: CallbackList = Vec::new();
    for callback in dynamic_list.iter().chain(global) {
        if !combined
            .iter()
            .any(|existing| Arc::ptr_eq(existing, callback))
        {
            combined.push(Arc::clone(callback));
        }
    }
    combined
}
