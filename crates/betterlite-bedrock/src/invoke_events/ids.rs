use betterlite_anthropic::stream_state::state;
use uuid::Uuid;

use crate::agentcore_stream;

pub(crate) struct IdFunction {
    pub(crate) next: fn() -> Uuid,
}

impl agentcore_stream::IdSource for IdFunction {
    fn next_id(&self) -> Uuid {
        (self.next)()
    }
}

impl state::IdSource for IdFunction {
    fn next_id(&self) -> Uuid {
        (self.next)()
    }
}
