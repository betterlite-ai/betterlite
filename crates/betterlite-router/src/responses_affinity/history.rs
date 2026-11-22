use betterlite_core::api::response_id_traversal::record::Record;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::message_content::Message;

use crate::deployment_identity::spec::DeploymentSpec;

// request_kwargs["litellm_metadata"] the way Python sees it: absent, assignable, or a value it cannot assign into.
pub enum MetadataSlot<'metadata> {
    Absent,
    Object(&'metadata mut JsonObject),
    NotAnObject { python_type: &'static str },
}

// The request state the check rewrites in place, so the router's fallback snapshot replays the stripped history.
pub struct History<'request> {
    // request_kwargs["input"], parsed once so the marker scan and the in-place strip share one tree
    pub input: Option<&'request mut Record>,
    // the messages argument
    pub messages: Option<&'request mut Vec<Message>>,
    // request_kwargs["messages"]
    pub request_messages: Option<&'request mut Vec<Message>>,
    pub metadata: MetadataSlot<'request>,
    // request_kwargs["_encrypted_content_affinity_pinned"]
    pub pinned: &'request mut bool,
}

impl History<'_> {
    // `messages or request_kwargs.get("messages")`: an explicit non-empty list wins, and an empty one falls back.
    #[must_use]
    pub fn effective_messages(&self) -> Option<&[Message]> {
        match self.messages.as_deref() {
            Some(messages) if !messages.is_empty() => Some(messages.as_slice()),
            Some(_) | None => self.request_messages.as_deref().map(Vec::as_slice),
        }
    }

    pub fn effective_messages_mut(&mut self) -> Option<&mut Vec<Message>> {
        match self.messages.as_deref_mut() {
            Some(messages) if !messages.is_empty() => Some(messages),
            Some(_) | None => self.request_messages.as_deref_mut(),
        }
    }
}

// The filter's arguments: the requested group, the already-authorized healthy pool it narrows, and the history it may strip.
pub struct FilterRequest<'request> {
    pub model: &'request str,
    pub deployments: &'request [DeploymentSpec],
    pub history: History<'request>,
}
