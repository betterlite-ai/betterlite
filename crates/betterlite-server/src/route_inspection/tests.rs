#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

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
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    let name = with_default(subscriber, || {
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
    assert!(captured.contains("ERROR"));
    assert!(captured.contains("broken endpoint"));
    drop(captured);
}

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buffer);
        self.0.lock().map_err(io::Error::other)?.push_str(&text);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
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
