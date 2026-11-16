use std::sync::Arc;

use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_redact::secret::Redactor;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::purview_graph_client::user_id_resolution::KeyIdentity;

// Where a scan's record goes: through the started invocation when one wraps the hook, and otherwise written by the scan itself.
pub struct RecordTargets<'targets> {
    pub base: &'targets GuardrailBase,
    pub invocation: Option<&'targets mut GuardrailInvocation>,
    pub sink: &'targets dyn GuardrailSpanSink,
    pub log_target: Option<&'targets mut GuardrailLogTarget>,
}

// What a blocking hook call needs from the dispatcher; the identity is the key or JWT bound user.
pub struct HookContext<'context> {
    pub identity: KeyIdentity<'context>,
    pub cache: Arc<DualLocalCache>,
    pub redactor: Arc<Redactor>,
    pub record_base: &'context GuardrailBase,
    pub invocation: Option<&'context mut GuardrailInvocation>,
    pub sink: &'context dyn GuardrailSpanSink,
    pub log_target: Option<&'context mut GuardrailLogTarget>,
}

impl HookContext<'_> {
    pub(super) fn targets(&mut self) -> RecordTargets<'_> {
        RecordTargets {
            base: self.record_base,
            invocation: self.invocation.as_deref_mut(),
            sink: self.sink,
            log_target: self.log_target.as_deref_mut(),
        }
    }
}
