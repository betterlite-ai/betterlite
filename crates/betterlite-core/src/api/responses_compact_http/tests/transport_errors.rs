use std::sync::Arc;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::response_events::ResponsesReply;

use super::support::{
    Events, FailureStage, Fixture, RecordingConfig, Reply, Script, Source, TestCall, Wire,
    compact_deps, openai_config,
};
use crate::api::responses_compact_http::config::Headers;
use crate::api::responses_compact_http::error::{CompactError, Stage};
use crate::api::responses_compact_http::service::{ClientKind, CompactRequest};
use crate::api::vector_store_http_write::error::HeaderMap;

pub(super) const STATUS_304_EVENTS: &[&str] = &[
    "validate",
    "url",
    "transform",
    "normalize",
    "sign",
    "log",
    "send",
];

pub(super) const SIGN_EVENTS: &[&str] = &["validate", "url", "transform", "normalize", "sign"];

pub(super) const VALIDATE_EVENTS: &[&str] = &["validate"];

pub(super) const TRANSFORM_EVENTS: &[&str] = &["validate", "url", "transform"];

pub(super) const LOG_EVENTS: &[&str] =
    &["validate", "url", "transform", "normalize", "sign", "log"];

pub(super) enum ConfigSpec {
    OpenAi(&'static str),
    Recording(&'static str),
    Scripted(&'static str, FailureStage),
    Transformed(&'static str, &'static str),
}

impl ConfigSpec {
    fn build(&self, events: &Events) -> RecordingConfig {
        match self {
            Self::OpenAi(key) => openai_config(key, events),
            Self::Recording(base) => RecordingConfig::new(base, events),
            Self::Scripted(base, stage) => {
                RecordingConfig::new(base, events).scripted(Script::Fail(*stage))
            }
            Self::Transformed(base, url) => {
                RecordingConfig::new(base, events).scripted(Script::TransformTo(String::from(*url)))
            }
        }
    }
}

pub(super) enum WireSpec {
    Reply(Reply),
    Closed,
}

pub(super) struct Scenario {
    pub(super) config: ConfigSpec,
    pub(super) wire: WireSpec,
    pub(super) mode: ClientKind,
    pub(super) headers: Option<Headers>,
    pub(super) expect: Expect,
}

pub(super) enum Expect {
    Provider(ProviderExpect),
    Step(StepExpect),
    RequestUrl(RequestUrlExpect),
    Header(HeaderExpect),
}

pub(super) struct ProviderExpect {
    pub(super) status: u16,
    pub(super) synthesized: bool,
    pub(super) message: String,
    pub(super) headers: HeaderMap,
    pub(super) logged: bool,
    pub(super) events: Option<&'static [&'static str]>,
}

pub(super) struct StepExpect {
    pub(super) stage: Stage,
    pub(super) text: String,
    pub(super) prefix: bool,
    pub(super) display: Option<&'static str>,
    pub(super) events: Option<&'static [&'static str]>,
    pub(super) sent: bool,
}

pub(super) struct RequestUrlExpect {
    pub(super) url: String,
    pub(super) events: Option<&'static [&'static str]>,
}

pub(super) enum HeaderExpect {
    Name {
        name: String,
        events: Option<&'static [&'static str]>,
    },
    Value {
        name: String,
    },
}

pub(super) struct Outcome {
    result: Result<ResponsesReply, CompactError>,
    requests: Vec<Arc<CompactRequest>>,
    errors: Vec<(String, u16, HeaderMap)>,
    events: Vec<String>,
}

pub(super) async fn run(scenario: &Scenario, base: &TestCall<'_>) -> Outcome {
    let events = Events::new();
    let config = scenario.config.build(&events);
    let wire = match &scenario.wire {
        WireSpec::Reply(reply) => Wire::replying(reply.clone(), &events),
        WireSpec::Closed => Wire::failing(TransportError::Closed, &events),
    };
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let mut call = TestCall::new(base.model, base.input, base.optional_params, base.params);
    call.extra_headers = scenario.headers.as_ref();
    let mut fixture = Fixture::new("responses");
    let result = fixture
        .run(&deps, scenario.mode, &call.call(), &events)
        .await;
    Outcome {
        result,
        requests: wire.requests(),
        errors: config.errors(),
        events: events.recorded(),
    }
}

pub(super) fn check(outcome: &Outcome, expect: &Expect) {
    match expect {
        Expect::Provider(provider) => provider.check(outcome),
        Expect::Step(step) => step.check(outcome),
        Expect::RequestUrl(url) => url.check(outcome),
        Expect::Header(HeaderExpect::Name { name, events }) => {
            let Err(CompactError::HeaderName { name: found, .. }) = &outcome.result else {
                panic!("a header name HTTP cannot carry is named in the error");
            };
            assert_eq!(found, name);
            assert!(outcome.requests.is_empty());
            check_events(outcome, *events);
        }
        Expect::Header(HeaderExpect::Value { name }) => {
            let Err(CompactError::HeaderValue { name: found, .. }) = &outcome.result else {
                panic!("a header value HTTP cannot carry is named in the error");
            };
            assert_eq!(found, name);
            assert!(outcome.requests.is_empty());
        }
    }
}

fn check_events(outcome: &Outcome, expected: Option<&'static [&'static str]>) {
    let Some(wanted) = expected else {
        return;
    };
    let recorded: Vec<&str> = outcome.events.iter().map(String::as_str).collect();
    assert_eq!(recorded.as_slice(), wanted);
}

impl ProviderExpect {
    fn check(&self, outcome: &Outcome) {
        let Err(CompactError::Provider(error)) = &outcome.result else {
            panic!("the reply is the provider error");
        };
        assert_eq!(error.status_code, self.status);
        assert_eq!(error.status_code_is_synthesized, self.synthesized);
        assert_eq!(error.message, self.message);
        assert_eq!(error.headers, self.headers);
        assert_eq!(outcome.requests.len(), 1);
        if self.logged {
            let entry = (self.message.clone(), self.status, self.headers.clone());
            assert_eq!(outcome.errors, vec![entry]);
        }
        check_events(outcome, self.events);
    }
}

impl StepExpect {
    fn check(&self, outcome: &Outcome) {
        let Err(error) = &outcome.result else {
            panic!("the failure is a typed step error");
        };
        let CompactError::Step {
            stage,
            source: failure,
        } = error
        else {
            panic!("the failure names its step");
        };
        assert_eq!(*stage, self.stage);
        if self.prefix {
            assert!(failure.to_string().starts_with(&self.text));
        } else {
            assert_eq!(failure.to_string(), self.text);
        }
        if let Some(display) = self.display {
            assert_eq!(error.to_string(), display);
        }
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.requests.len(), usize::from(self.sent));
        check_events(outcome, self.events);
    }
}

impl RequestUrlExpect {
    fn check(&self, outcome: &Outcome) {
        let Err(CompactError::RequestUrl { url, .. }) = &outcome.result else {
            panic!("the transformed URL is named in the error");
        };
        assert_eq!(url, &self.url);
        assert!(outcome.requests.is_empty());
        check_events(outcome, self.events);
    }
}
