use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::batch_enqueued_refund::store::ReservationRecordScripts;
use betterlite_core::api::batch_enqueued_reserve::store::{ReservationScripts, ReserveScriptReply};
use serde_json::{Value, json};

use super::{PopStep, ScriptsInput, Step};

#[derive(Debug, thiserror::Error)]
#[error("the scripted script call failed: {message}")]
pub(super) struct ScriptFailure {
    pub(super) message: String,
}

pub(super) struct ScriptedScripts {
    pub(super) events: Arc<Mutex<Vec<Value>>>,
    refunds: Mutex<VecDeque<Step>>,
    saves: Mutex<VecDeque<Step>>,
    pops: Mutex<VecDeque<PopStep>>,
}

impl ScriptedScripts {
    pub(super) fn new(events: Arc<Mutex<Vec<Value>>>, spec: ScriptsInput) -> Self {
        Self {
            events,
            refunds: Mutex::new(VecDeque::from(spec.refund)),
            saves: Mutex::new(VecDeque::from(spec.save)),
            pops: Mutex::new(VecDeque::from(spec.pop)),
        }
    }

    fn push_event(&self, event: Value) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn take_step(&self, queue: &Mutex<VecDeque<Step>>, event: Value) -> Result<(), ScriptFailure> {
        self.push_event(event);
        self.take_reply(queue)
    }

    fn take_reply(&self, queue: &Mutex<VecDeque<Step>>) -> Result<(), ScriptFailure> {
        let step = queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match step {
            Some(Step::Done) => Ok(()),
            Some(Step::Failure { message }) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted queue is empty".to_owned(),
            }),
        }
    }
}

impl ReservationScripts for ScriptedScripts {
    type Error = ScriptFailure;

    async fn reserve(
        &self,
        _counter_key: &str,
        _tokens: i64,
        _ttl_seconds: i64,
        _limit: i64,
    ) -> Result<ReserveScriptReply, ScriptFailure> {
        Err(ScriptFailure {
            message: "the refund helper never reserves".to_owned(),
        })
    }

    async fn refund(&self, counter_key: &str, tokens: i64) -> Result<(), ScriptFailure> {
        let event = json!({"event": "refund", "key": counter_key, "args": [tokens]});
        self.take_step(&self.refunds, event)
    }
}

impl ReservationRecordScripts for ScriptedScripts {
    async fn save(
        &self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> Result<(), ScriptFailure> {
        let stored = serde_json::from_str::<Value>(record).unwrap_or(Value::Null);
        self.push_event(
            json!({"event": "save", "key": record_key, "record": stored, "args": [ttl_seconds]}),
        );
        self.take_reply(&self.saves)
    }

    async fn pop(
        &self,
        record_key: &str,
        ttl_seconds: i64,
    ) -> Result<Option<Vec<u8>>, ScriptFailure> {
        self.push_event(json!({"event": "pop", "key": record_key, "args": [ttl_seconds]}));
        let step = self
            .pops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match step {
            Some(PopStep::Record { record }) => {
                serde_json::to_vec(&record)
                    .map(Some)
                    .map_err(|error| ScriptFailure {
                        message: error.to_string(),
                    })
            }
            Some(PopStep::Tombstone) => Ok(Some(Vec::new())),
            Some(PopStep::Absent) => Ok(None),
            Some(PopStep::Failure { message }) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted pop queue is empty".to_owned(),
            }),
        }
    }
}
