#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;

#[derive(Default)]
struct TestSpan {
    tags: Mutex<Vec<(String, String)>>,
    trace: Option<String>,
    span: Option<String>,
}

impl Span for TestSpan {
    fn set_tag_str(&self, key: &str, value: &str) -> bool {
        self.tags
            .lock()
            .expect("test mutex is not poisoned")
            .push((key.to_owned(), value.to_owned()));
        true
    }

    fn trace_id(&self) -> Option<String> {
        self.trace.clone()
    }

    fn span_id(&self) -> Option<String> {
        self.span.clone()
    }
}

struct TestTracer {
    current: Option<Arc<TestSpan>>,
    root: Option<Arc<TestSpan>>,
}

impl Tracer for TestTracer {
    fn current_span(&self) -> Option<Arc<dyn Span>> {
        self.current.as_ref().map(Arc::clone).map(|test_span| {
            let trait_span: Arc<dyn Span> = test_span;
            trait_span
        })
    }

    fn current_root_span(&self) -> Option<Arc<dyn Span>> {
        self.root.as_ref().map(Arc::clone).map(|test_span| {
            let trait_span: Arc<dyn Span> = test_span;
            trait_span
        })
    }
}

#[test]
fn datadog_tag_normalization_behavior() {
    assert_eq!(normalize_datadog_tag_value(" Foo.Bar "), "foo.bar");
    assert_eq!(normalize_datadog_tag_value("A__B / C"), "a_b_/_c");
    assert_eq!(normalize_datadog_tag_value("__\u{c4}__"), "\u{e4}");
}

#[test]
fn datadog_environment_defaults_behavior() {
    let values = BTreeMap::from([(String::from("DD_SERVICE"), String::from("api"))]);
    let environment = DatadogEnvironment::from_lookup(&values);
    assert_eq!(environment.service, "api");
    assert_eq!(environment.source, "litellm");
    assert_eq!(environment.base_url, None);
}

#[test]
fn datadog_span_context_behavior() {
    let current = Arc::new(TestSpan {
        trace: Some(String::from("12")),
        span: None,
        tags: Mutex::new(Vec::new()),
    });
    let root = Arc::new(TestSpan {
        trace: Some(String::from("root")),
        span: Some(String::from("34")),
        tags: Mutex::new(Vec::new()),
    });
    let tracer = TestTracer {
        current: Some(current),
        root: Some(root),
    };
    assert_eq!(
        active_trace_context(&tracer).expect("context"),
        BTreeMap::from([(String::from("trace_id"), String::from("12")),])
    );
}

#[test]
fn datadog_proxy_span_tagging_behavior() {
    let span = Arc::new(TestSpan::default());
    let tracer = TestTracer {
        current: Some(Arc::clone(&span)),
        root: None,
    };
    tag_call_id(&tracer, Some(String::from("call")));
    tag_request(
        &tracer,
        Some(String::from("alias")),
        Some(String::from("hash")),
        Some(String::from("model")),
    );
    assert_eq!(span.tags.lock().expect("test mutex").len(), 4);
}
