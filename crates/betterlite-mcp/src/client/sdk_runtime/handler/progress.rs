use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use rmcp::model::{ProgressNotificationParam, ProgressToken};
use tokio::sync::mpsc;

const BUFFER: usize = 16;
const EARLY: usize = 64;

// The session learns a call's progress token only once the request is on the wire, so notifications that arrive sooner wait here
#[derive(Default)]
pub struct ProgressRegistry {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    waiters: HashMap<ProgressToken, mpsc::Sender<ProgressNotificationParam>>,
    early: VecDeque<(ProgressToken, ProgressNotificationParam)>,
}

impl ProgressRegistry {
    pub fn subscribe(&self, token: &ProgressToken) -> mpsc::Receiver<ProgressNotificationParam> {
        let (sender, receiver) = mpsc::channel(BUFFER);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut kept = VecDeque::with_capacity(state.early.len());
        while let Some((pending, param)) = state.early.pop_front() {
            if &pending == token {
                // a full buffer drops the update, like every later notification for a slow consumer
                drop(sender.try_send(param));
            } else {
                kept.push_back((pending, param));
            }
        }
        state.early = kept;
        state.waiters.insert(token.clone(), sender);
        drop(state);
        receiver
    }

    pub fn unsubscribe(&self, token: &ProgressToken) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.waiters.remove(token);
    }

    pub fn deliver(&self, param: ProgressNotificationParam) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sender) = state.waiters.get(&param.progress_token) else {
            if state.early.len() == EARLY {
                state.early.pop_front();
            }
            state.early.push_back((param.progress_token.clone(), param));
            return;
        };
        drop(sender.try_send(param));
    }
}
