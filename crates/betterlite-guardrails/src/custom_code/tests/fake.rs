use std::collections::BTreeMap;
use std::future::pending as pending_future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::ThreadId;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::custom_code::CallbackRegistry;
use crate::custom_code::CustomCodeGuardrail;
use crate::custom_code::config::CustomCodeGuardrailParams;
use crate::custom_code::config::DEFAULT_GUARDRAIL_NAME;
use crate::custom_code::inputs::CustomCodeInputs;
use crate::custom_code::runtime::CallArguments;
use crate::custom_code::runtime::CallOutput;
use crate::custom_code::runtime::LoadFailure;
use crate::custom_code::runtime::LoadedModule;
use crate::custom_code::runtime::ModuleHost;
use crate::custom_code::runtime::RuntimeFailure;
use crate::custom_code_execution::deadline;
use crate::custom_code_execution::execution::error::SandboxError;
use crate::custom_code_execution::execution::error::SandboxExit;
use crate::custom_code_execution::sandbox::CompiledProgram;
use crate::custom_code_execution::sandbox::globals::SandboxGlobals;

pub(super) const ALLOW_SOURCE: &str =
    "def apply_guardrail(inputs, request_data, input_type):\n    return allow()\n";
pub(super) const ASYNC_SOURCE: &str =
    "async def apply_guardrail(inputs, request_data, input_type):\n    return allow()\n";

pub(super) type Behaviour =
    Arc<dyn Fn(&CallArguments) -> Result<CallOutput, SandboxError<RuntimeFailure>> + Send + Sync>;

#[derive(Default)]
pub(super) struct CallLog {
    pub calls: AtomicUsize,
    pub seen: Mutex<Option<CallArguments>>,
    pub threads: Mutex<Vec<ThreadId>>,
}

pub(super) struct Scripted {
    log: Arc<CallLog>,
    behaviour: Behaviour,
}

impl LoadedModule for Scripted {
    fn call(&self, arguments: CallArguments) -> Result<CallOutput, SandboxError<RuntimeFailure>> {
        self.log.calls.fetch_add(1, Ordering::SeqCst);
        self.log
            .threads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(thread::current().id());
        *self.log.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(arguments.clone());
        (self.behaviour.as_ref())(&arguments)
    }
}

pub(super) enum LoadReply {
    Module(Behaviour),
    MissingFunction,
    NotCallable,
    Failure(&'static str),
    Stuck,
}

pub(super) struct FakeHost {
    pub loads: AtomicUsize,
    pub log: Arc<CallLog>,
    pub sources: Mutex<Vec<String>>,
    pub globals_seen: Mutex<Option<SandboxGlobals>>,
    pub reply: LoadReply,
}

impl FakeHost {
    pub(super) fn new(reply: LoadReply) -> Arc<Self> {
        Arc::new(Self {
            loads: AtomicUsize::new(0),
            log: Arc::new(CallLog::default()),
            sources: Mutex::new(Vec::new()),
            globals_seen: Mutex::new(None),
            reply,
        })
    }

    pub(super) fn load_count(&self) -> usize {
        self.loads.load(Ordering::SeqCst)
    }
}

impl ModuleHost for FakeHost {
    fn load(
        &self,
        program: &CompiledProgram,
        globals: &SandboxGlobals,
    ) -> Result<Box<dyn LoadedModule>, SandboxError<LoadFailure>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(program.source.clone());
        *self
            .globals_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(globals.clone());
        match &self.reply {
            LoadReply::Module(behaviour) => Ok(Box::new(Scripted {
                log: Arc::clone(&self.log),
                behaviour: Arc::clone(behaviour),
            })),
            LoadReply::MissingFunction => Err(SandboxError::Failed(LoadFailure::MissingFunction)),
            LoadReply::NotCallable => Err(SandboxError::Failed(LoadFailure::NotCallable)),
            LoadReply::Failure(message) => {
                Err(SandboxError::Failed(LoadFailure::Runtime(RuntimeFailure {
                    message: (*message).to_owned(),
                })))
            }
            LoadReply::Stuck => loop {
                if let Err(interrupted) = deadline::budget_ok() {
                    return Err(SandboxError::Interrupted(interrupted));
                }
            },
        }
    }
}

#[derive(Default)]
pub(super) struct FakeRegistry {
    pub callbacks: Mutex<Vec<Arc<CustomCodeGuardrail>>>,
}

impl CallbackRegistry for FakeRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<CustomCodeGuardrail>) {
        self.callbacks.lock().unwrap().push(guardrail);
    }
}

pub(super) fn fixed_clock() -> Duration {
    Duration::from_millis(5)
}

pub(super) fn wall_clock() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

// Starts at 1 s and advances 10 ms per read, so a hardcoded start or a reused read stands out.
pub(super) fn ticking_clock() -> Duration {
    static NEXT: AtomicU64 = AtomicU64::new(1000);
    Duration::from_millis(NEXT.fetch_add(10, Ordering::SeqCst))
}

pub(super) fn as_host(host: &Arc<FakeHost>) -> Arc<dyn ModuleHost> {
    let cloned = Arc::clone(host);
    let dynamic: Arc<dyn ModuleHost> = cloned;
    dynamic
}

pub(super) fn params(source: &str) -> CustomCodeGuardrailParams {
    CustomCodeGuardrailParams {
        custom_code: Some(source.to_owned()),
        guardrail_name: Some(DEFAULT_GUARDRAIL_NAME.to_owned()),
        timeout: None,
    }
}

pub(super) fn params_with_timeout(source: &str, timeout: f64) -> CustomCodeGuardrailParams {
    CustomCodeGuardrailParams {
        timeout: Some(timeout),
        ..params(source)
    }
}

pub(super) fn build(
    params: CustomCodeGuardrailParams,
    host: &Arc<FakeHost>,
) -> CustomCodeGuardrail {
    build_with_clock(params, host, fixed_clock)
}

pub(super) fn build_with_clock(
    params: CustomCodeGuardrailParams,
    host: &Arc<FakeHost>,
    clock: fn() -> Duration,
) -> CustomCodeGuardrail {
    CustomCodeGuardrail::new(params, as_host(host), clock).unwrap()
}

pub(super) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(super) fn object(entries: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(super) fn map(entries: Vec<(&str, GuardrailValue)>) -> BTreeMap<String, GuardrailValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn inputs(texts: &[&str]) -> CustomCodeInputs {
    CustomCodeInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tool_calls: None,
        extra: BTreeMap::new(),
    }
}

pub(super) fn value(result: GuardrailValue) -> Behaviour {
    Arc::new(move |_arguments| Ok(CallOutput::Value(result.clone())))
}

pub(super) fn pending(result: GuardrailValue) -> Behaviour {
    Arc::new(move |_arguments| {
        let reply_value = result.clone();
        Ok(CallOutput::Pending(Box::pin(async move {
            Ok::<GuardrailValue, SandboxError<RuntimeFailure>>(reply_value)
        })))
    })
}

pub(super) fn never() -> Behaviour {
    Arc::new(|_arguments| {
        Ok(CallOutput::Pending(Box::pin(pending_future::<
            Result<GuardrailValue, SandboxError<RuntimeFailure>>,
        >())))
    })
}

pub(super) fn failing(message: &'static str) -> Behaviour {
    Arc::new(move |_arguments| {
        Err(SandboxError::Failed(RuntimeFailure {
            message: message.to_owned(),
        }))
    })
}

pub(super) fn exiting() -> Behaviour {
    Arc::new(|_arguments| {
        Err(SandboxError::Exit(SandboxExit::system_exit(
            "bye".to_owned(),
        )))
    })
}

pub(super) fn spinning() -> Behaviour {
    Arc::new(|_arguments| {
        loop {
            if let Err(interrupted) = deadline::budget_ok() {
                return Err(SandboxError::Interrupted(interrupted));
            }
        }
    })
}

pub(super) fn parking_then_pending(duration: Duration) -> Behaviour {
    Arc::new(move |_arguments| {
        thread::park_timeout(duration);
        Ok(CallOutput::Pending(Box::pin(pending_future::<
            Result<GuardrailValue, SandboxError<RuntimeFailure>>,
        >())))
    })
}
