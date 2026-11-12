use super::records::ChirpStreamEvent;

#[derive(Clone, Debug)]
pub enum OutboxItem {
    Text(String),
    TurnResult {
        turn: usize,
        event: ChirpStreamEvent,
    },
    TurnDiscardedEvent {
        turn: usize,
        event: String,
    },
    StreamFailure {
        reason: String,
    },
    Closed,
}
