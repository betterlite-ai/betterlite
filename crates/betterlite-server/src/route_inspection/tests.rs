#![cfg(test)]

use std::fmt::Debug;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use tracing::{Event, Subscriber, field::Field};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

use super::{
    NamedEndpoint, Route, RouteInfo, RouteInspectionError, get_app_routes,
    get_routes_for_mounted_app,
};

struct Endpoint {
    name: Option<&'static str>,
    class_name: Option<&'static str>,
    fail: bool,
}

impl NamedEndpoint for Endpoint {
    fn endpoint_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        if self.fail {
            Err(RouteInspectionError::EndpointName)
        } else {
            Ok(self.name)
        }
    }

    fn class_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(self.class_name)
    }
}

#[test]
fn regular_route_keeps_nulls_and_endpoint_name() {
    let route = Route {
        path: Some("/chat".to_owned()),
        methods: Some(Some(vec!["POST".to_owned()])),
        name: Some("chat".to_owned()),
        endpoint: Some("present".to_owned()),
        app_name: None,
        app: None,
    };
    let info = get_app_routes(
        &route,
        &Endpoint {
            name: Some("handler"),
            class_name: None,
            fail: false,
        },
    );
    assert_eq!(info[0].endpoint.as_deref(), Some("handler"));
    assert_eq!(info[0].mounted_app, None);
}

#[test]
fn regular_route_without_endpoint_keeps_null_fields() {
    let info = get_app_routes(
        &Route::default(),
        &Endpoint {
            name: Some("handler"),
            class_name: None,
            fail: false,
        },
    );
    assert_eq!(
        info,
        vec![RouteInfo {
            path: None,
            methods: None,
            name: None,
            endpoint: None,
            mounted_app: None,
        }]
    );
}

#[test]
fn mounted_routes_join_prefix_and_default_methods() {
    let route = Route {
        path: Some("/v1/".to_owned()),
        app_name: None,
        app: Some(vec![Route {
            path: Some("/models".to_owned()),
            endpoint: Some("list_models".to_owned()),
            methods: None,
            name: None,
            app_name: None,
            app: None,
        }]),
        methods: None,
        name: None,
        endpoint: None,
    };
    let info = get_routes_for_mounted_app(&route);
    assert_eq!(info[0].path.as_deref(), Some("/v1/models"));
    assert_eq!(
        info[0].methods,
        Some(vec!["GET".to_owned(), "POST".to_owned()])
    );
    assert_eq!(info[0].mounted_app, Some(true));
}

#[test]
fn mounted_app_with_no_routes_is_empty() {
    assert!(get_routes_for_mounted_app(&Route::default()).is_empty());
}

#[test]
fn mounted_routes_skip_routes_without_endpoint_or_nested_app() {
    let route = Route {
        path: None,
        methods: None,
        name: None,
        endpoint: None,
        app_name: None,
        app: Some(vec![Route {
            path: None,
            methods: None,
            name: None,
            endpoint: None,
            app_name: None,
            app: None,
        }]),
    };
    assert!(get_routes_for_mounted_app(&route).is_empty());
}

#[test]
fn endpoint_name_error_is_logged_and_dropped() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturedEvents(Arc::clone(&records)));
    let name = tracing::subscriber::with_default(subscriber, || {
        super::safe_get_endpoint_name(
            "broken endpoint",
            &Endpoint {
                name: None,
                class_name: None,
                fail: true,
            },
        )
    });
    assert_eq!(name, None);
    let captured = records.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("broken endpoint"));
}

struct CapturedEvents(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for CapturedEvents {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            let mut fields = EventFields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }
}

struct EventFields(String);

impl tracing::field::Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

#[test]
fn endpoint_name_falls_back_to_its_class_name() {
    let name = super::safe_get_endpoint_name(
        "class endpoint",
        &Endpoint {
            name: None,
            class_name: Some("Handler"),
            fail: false,
        },
    );
    assert_eq!(name.as_deref(), Some("Handler"));
}

#[test]
fn mounted_route_with_explicit_null_methods_keeps_null() {
    let route = Route {
        path: None,
        methods: None,
        name: None,
        endpoint: None,
        app_name: None,
        app: Some(vec![Route {
            path: Some("/null-methods".to_owned()),
            methods: Some(None),
            name: None,
            endpoint: Some("handler".to_owned()),
            app_name: None,
            app: None,
        }]),
    };
    assert_eq!(get_routes_for_mounted_app(&route)[0].methods, None);
}

#[test]
fn mounted_nested_app_uses_its_class_name() {
    let route = Route {
        path: Some("/mounted".to_owned()),
        methods: None,
        name: None,
        endpoint: None,
        app_name: None,
        app: Some(vec![Route {
            path: Some("/files".to_owned()),
            methods: None,
            name: None,
            endpoint: None,
            app_name: Some("StaticFiles".to_owned()),
            app: Some(Vec::new()),
        }]),
    };
    assert_eq!(
        get_routes_for_mounted_app(&route)[0].endpoint.as_deref(),
        Some("StaticFiles")
    );
}
