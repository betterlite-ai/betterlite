use std::collections::BTreeMap;
use std::io::Error;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Barrier;
use tokio::time;

use super::{CallOutcome, MAX_TRACE, MockScript, Outcome, ProviderCall, Wait};
use crate::mock::ProviderRequest;

pub(crate) struct Recording {
    script: MockScript,
    run: usize,
    barriers: BTreeMap<String, Arc<Barrier>>,
    trace: Vec<ProviderCall>,
    pub(crate) failure: Option<Error>,
}

pub(crate) struct Ticket {
    pub(crate) run: usize,
    pub(crate) index: usize,
    pause: Pause,
    deadline: Duration,
    pub(crate) then: Outcome,
}

enum Pause {
    None,
    Delay(Duration),
    Barrier(Arc<Barrier>),
}

pub(crate) struct CallGuard {
    pub(crate) state: Arc<Mutex<Recording>>,
    pub(crate) run: usize,
    pub(crate) index: usize,
    pub(crate) outcome: CallOutcome,
}

impl Recording {
    pub(crate) const fn new(script: MockScript) -> Self {
        Self {
            script,
            barriers: BTreeMap::new(),
            run: 0,
            trace: Vec::new(),
            failure: None,
        }
    }

    pub(crate) fn admit(&mut self, request: ProviderRequest) -> Result<Ticket, Error> {
        if self.trace.len() >= MAX_TRACE {
            return Err(Error::other(format!(
                "provider trace exceeded {MAX_TRACE} calls"
            )));
        }
        let index = self.trace.len();
        let Some(reply) = self.script.replies.get(index) else {
            self.trace.push(ProviderCall {
                request,
                outcome: CallOutcome::Exhausted,
            });
            return Err(Error::other("provider answer sequence exhausted"));
        };
        let pause = pause(&mut self.barriers, &self.script.barriers, &reply.wait)?;
        let then = reply.then.clone();
        self.trace.push(ProviderCall {
            request,
            outcome: CallOutcome::Waiting,
        });
        Ok(Ticket {
            run: self.run,
            index,
            pause,
            deadline: self.script.deadline,
            then,
        })
    }

    pub(crate) fn finish_run(&mut self) -> Result<Vec<ProviderCall>, Error> {
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        self.run = self.run.saturating_add(1);
        self.barriers.clear();
        Ok(mem::take(&mut self.trace))
    }
}

impl Ticket {
    pub(crate) async fn wait(&self) -> bool {
        match &self.pause {
            Pause::None => true,
            Pause::Delay(delay) => {
                time::sleep(*delay).await;
                true
            }
            Pause::Barrier(barrier) => time::timeout(self.deadline, barrier.wait()).await.is_ok(),
        }
    }
}

fn pause(
    barriers: &mut BTreeMap<String, Arc<Barrier>>,
    parties: &BTreeMap<String, usize>,
    wait: &Wait,
) -> Result<Pause, Error> {
    match wait {
        Wait::None => Ok(Pause::None),
        Wait::Delay { millis } => Ok(Pause::Delay(Duration::from_millis(*millis))),
        Wait::Barrier { name } => {
            let count = *parties
                .get(name)
                .ok_or_else(|| Error::other(format!("barrier {name} is not in the script")))?;
            let barrier = barriers
                .entry(name.clone())
                .or_insert_with(|| Arc::new(Barrier::new(count)));
            Ok(Pause::Barrier(Arc::clone(barrier)))
        }
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if let Ok(mut recorded) = self.state.lock()
            && recorded.run == self.run
            && let Some(call) = recorded.trace.get_mut(self.index)
        {
            call.outcome = self.outcome;
        }
    }
}
