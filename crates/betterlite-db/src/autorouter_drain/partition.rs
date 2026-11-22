use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnTime};

// _session_partition: a nonempty api_key gives the key its own history, and a turn without one partitions by user.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DrainPartition {
    Key {
        api_key: String,
        session_id: String,
        router_name: String,
    },
    User {
        user_id: String,
        session_id: String,
        router_name: String,
    },
}

#[must_use]
pub fn partition_of(turn: &AutoRouterTurnTransaction) -> DrainPartition {
    if turn.api_key.is_empty() {
        DrainPartition::User {
            user_id: turn.user_id.clone(),
            session_id: turn.session_id.clone(),
            router_name: turn.router_name.clone(),
        }
    } else {
        DrainPartition::Key {
            api_key: turn.api_key.clone(),
            session_id: turn.session_id.clone(),
            router_name: turn.router_name.clone(),
        }
    }
}

#[must_use]
pub fn turn_micros(time: TurnTime) -> i64 {
    match time {
        TurnTime::Aware(time) => time.timestamp_micros(),
        TurnTime::Naive(time) => time.and_utc().timestamp_micros(),
    }
}
