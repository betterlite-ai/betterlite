#![expect(
    clippy::disallowed_types,
    reason = "the slot carries whichever typed value its consumer armed, like LiteLLM's dict[str, object], and each consumer downcasts its own type when it takes the slot"
)]

use std::any::Any;

pub type PrefetchSlot = Box<dyn Any + Send + Sync>;
