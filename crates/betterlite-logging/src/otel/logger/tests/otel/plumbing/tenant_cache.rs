#![cfg(test)]

mod detach {
    use std::collections::BTreeMap;

    use crate::otel::support::{
        Built, cache, creds, default_tracer, destination, owned_http, phoenix_http,
    };

    #[test]
    fn detach_is_true_for_credential_and_project_routes() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let credential = tenants
            .route_for(&default, Some(&creds("S")), None, &[])
            .expect("route");
        assert!(credential.detached);
        let project_built = Built::default();
        let projects = cache(phoenix_http(), Some("arize_phoenix"), &project_built);
        let auth = BTreeMap::from([("phoenix_project_name".to_owned(), "proj".to_owned())]);
        let project = projects
            .route_for(&default, None, Some(&auth), &[])
            .expect("route");
        assert!(project.detached);
    }

    #[test]
    fn detach_is_false_for_a_service_name_only_route() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let auth = BTreeMap::from([("otel_service_name".to_owned(), "payments".to_owned())]);
        let route = tenants
            .route_for(&default, None, Some(&auth), &[])
            .expect("route");
        assert!(route.provider.is_some());
        assert!(!route.detached);
    }

    #[test]
    fn detach_is_false_when_the_callback_already_has_a_destination() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let destinations = vec![destination("arize")];
        let route = tenants
            .route_for(&default, Some(&creds("S")), None, &destinations)
            .expect("route");
        // the request already names a backend for this callback, so the tenants' route is bypassed
        assert!(route.provider.is_none());
        assert!(!route.detached);
        assert_eq!(built.builds(), 0);
    }
}

mod eviction {
    use std::sync::Arc;

    use betterlite_logging::otel::plumbing::routing::cache::{
        MAX_CACHED_PROVIDERS, MAX_RETIRED_PROVIDERS,
    };

    use crate::otel::support::{Built, cache, creds, default_tracer, owned_http};

    #[test]
    fn eviction_drains_an_evicted_provider_until_its_last_lease_releases() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let first = tenants
            .route_for(&default, Some(&creds("S0")), None, &[])
            .expect("route");
        for index in 1..=MAX_CACHED_PROVIDERS {
            let space = format!("S{index}");
            tenants
                .route_for(&default, Some(&creds(&space)), None, &[])
                .expect("route");
        }
        assert_eq!(built.builds(), MAX_CACHED_PROVIDERS.saturating_add(1));
        // S0 was the oldest and still carried its route's lease: it drains rather than shutting down
        let drained = built.sink(0);
        assert!(!drained.is_shutdown_called());
        tenants.release(first.provider.as_ref());
        assert!(drained.is_shutdown_called());
    }

    #[test]
    fn eviction_shuts_down_a_provider_with_no_open_spans_at_once() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let first = tenants
            .route_for(&default, Some(&creds("S0")), None, &[])
            .expect("route");
        tenants.release(first.provider.as_ref());
        for index in 1..=MAX_CACHED_PROVIDERS {
            let space = format!("S{index}");
            tenants
                .route_for(&default, Some(&creds(&space)), None, &[])
                .expect("route");
        }
        // S0 had no open spans left, so the overflow eviction shuts it down inside route_for
        assert!(built.sink(0).is_shutdown_called());
    }

    #[test]
    fn eviction_shuts_down_the_stalest_retiree_past_the_retired_cap() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let total = MAX_CACHED_PROVIDERS
            .saturating_add(MAX_RETIRED_PROVIDERS)
            .saturating_add(1);
        for index in 0..total {
            let space = format!("S{index}");
            tenants
                .route_for(&default, Some(&creds(&space)), None, &[])
                .expect("route");
        }
        assert_eq!(built.builds(), total);
        // every provider here is still leased, so each eviction retires; the 65th retiree sheds the
        // stalest one to keep the retired set bounded
        assert!(built.sink(0).is_shutdown_called());
    }

    #[test]
    fn eviction_keeps_the_newest_provider_cached() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let mut newest = None;
        for index in 0..=MAX_CACHED_PROVIDERS {
            let space = format!("S{index}");
            let route = tenants
                .route_for(&default, Some(&creds(&space)), None, &[])
                .expect("route");
            newest = route.provider;
        }
        let builds = built.builds();
        let route = tenants
            .route_for(&default, Some(&creds("S256")), None, &[])
            .expect("route");
        assert_eq!(built.builds(), builds);
        assert!(Arc::ptr_eq(
            newest.as_ref().expect("newest"),
            route.provider.as_ref().expect("provider")
        ));
    }
}

mod headers {
    use std::collections::BTreeMap;

    use betterlite_logging::otel::config::ExporterOwner;
    use betterlite_logging::otel::config::destination::SpanScope;

    use crate::otel::support::{Built, cache, settings, spec, with_exporters};

    fn credentials() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("api_key".to_owned(), "K".to_owned()),
            ("arize-space-id".to_owned(), "S".to_owned()),
        ])
    }

    #[test]
    fn headers_replace_the_static_credentials_of_an_owned_exporter() {
        let built = Built::default();
        let tenants = cache(
            settings(
                "otlp_http",
                Some(ExporterOwner::ArizeAx),
                Some("space=static"),
                None,
            ),
            Some("arize"),
            &built,
        );
        let routed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
        assert_eq!(
            routed
                .exporters
                .first()
                .expect("exporter")
                .headers
                .as_deref(),
            Some("api_key=K,arize-space-id=S")
        );
    }

    #[test]
    fn headers_append_percent_encoded_project_headers() {
        let built = Built::default();
        let tenants = cache(
            settings(
                "otlp_http",
                Some(ExporterOwner::ArizePhoenix),
                Some("Authorization=Bearer k"),
                Some("http://phoenix:6006"),
            ),
            Some("arize_phoenix"),
            &built,
        );
        let project = BTreeMap::from([("x-project-name".to_owned(), "my proj, prod=1".to_owned())]);
        let routed = tenants.routed_config(&BTreeMap::new(), &project, None, None);
        assert_eq!(
            routed
                .exporters
                .first()
                .expect("exporter")
                .headers
                .as_deref(),
            Some("Authorization=Bearer k,x-project-name=my%20proj%2C%20prod%3D1")
        );
    }

    #[test]
    fn headers_keep_another_owners_exporter_untouched() {
        let built = Built::default();
        let tenants = cache(
            with_exporters(vec![
                spec(
                    "otlp_http",
                    Some(ExporterOwner::ArizeAx),
                    Some("space=base"),
                    None,
                ),
                spec("otlp_http", None, Some("x=collector"), None),
            ]),
            Some("arize"),
            &built,
        );
        let routed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
        assert_eq!(
            routed
                .exporters
                .first()
                .expect("exporter")
                .headers
                .as_deref(),
            Some("api_key=K,arize-space-id=S")
        );
        assert_eq!(
            routed
                .exporters
                .get(1)
                .expect("collector")
                .headers
                .as_deref(),
            Some("x=collector")
        );
    }

    #[test]
    fn headers_keep_project_headers_off_grpc_but_still_replace_credentials() {
        let built = Built::default();
        let tenants = cache(
            settings(
                "otlp_grpc",
                Some(ExporterOwner::ArizePhoenix),
                Some("space=base"),
                Some("http://localhost:4317"),
            ),
            Some("arize_phoenix"),
            &built,
        );
        let project = BTreeMap::from([("x-project-name".to_owned(), "proj".to_owned())]);
        let routed = tenants.routed_config(&BTreeMap::new(), &project, None, None);
        assert_eq!(
            routed
                .exporters
                .first()
                .expect("exporter")
                .headers
                .as_deref(),
            Some("space=base")
        );
        let credentialed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
        assert_eq!(
            credentialed
                .exporters
                .first()
                .expect("exporter")
                .headers
                .as_deref(),
            Some("api_key=K,arize-space-id=S")
        );
    }

    #[test]
    fn headers_skip_a_headerless_exporter() {
        let built = Built::default();
        let tenants = cache(
            settings(
                "in_memory",
                Some(ExporterOwner::ArizeAx),
                Some("space=base"),
                None,
            ),
            Some("arize"),
            &built,
        );
        let routed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
        assert_eq!(
            routed
                .exporters
                .first()
                .expect("exporter")
                .headers
                .as_deref(),
            Some("space=base")
        );
    }

    #[test]
    fn headers_apply_the_tenant_endpoint_and_service_name() {
        let mut routed_settings = settings(
            "otlp_http",
            Some(ExporterOwner::ArizeAx),
            None,
            Some("http://static:4318"),
        );
        routed_settings.langfuse_span_scope = SpanScope::LlmOnly;
        let built = Built::default();
        let tenants = cache(routed_settings, Some("arize"), &built);
        let routed = tenants.routed_config(
            &BTreeMap::new(),
            &BTreeMap::new(),
            Some("http://tenant:4318"),
            Some("payments"),
        );
        assert_eq!(routed.service_name, "payments");
        // a routed config always exports full spans, whatever the static setting said
        assert_eq!(routed.langfuse_span_scope, SpanScope::Full);
        assert_eq!(
            routed
                .exporters
                .first()
                .expect("exporter")
                .endpoint
                .as_deref(),
            Some("http://tenant:4318")
        );
    }
}

mod routing {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::otel::support::{Built, cache, creds, default_tracer, owned_http, phoenix_http};

    #[test]
    fn routing_caches_one_provider_per_credential_set() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let first = tenants
            .route_for(&default, Some(&creds("S1")), None, &[])
            .expect("route");
        let again = tenants
            .route_for(&default, Some(&creds("S1")), None, &[])
            .expect("route");
        let other = tenants
            .route_for(&default, Some(&creds("S2")), None, &[])
            .expect("route");
        assert!(first.detached);
        assert!(other.detached);
        assert_eq!(built.builds(), 2);
        assert!(Arc::ptr_eq(
            first.provider.as_ref().expect("first"),
            again.provider.as_ref().expect("again")
        ));
        assert!(!Arc::ptr_eq(
            first.provider.as_ref().expect("first"),
            other.provider.as_ref().expect("other")
        ));
    }

    #[test]
    fn routing_returns_the_default_tracer_without_a_tenant_signal() {
        let built = Built::default();
        let tenants = cache(owned_http(), Some("arize"), &built);
        let default = default_tracer();
        let route = tenants.route_for(&default, None, None, &[]).expect("route");
        assert!(route.provider.is_none());
        assert!(!route.detached);
        assert_eq!(built.builds(), 0);
    }

    #[test]
    fn routing_keeps_one_provider_per_project() {
        let built = Built::default();
        let tenants = cache(phoenix_http(), Some("arize_phoenix"), &built);
        let default = default_tracer();
        let first = BTreeMap::from([("phoenix_project_name".to_owned(), "a".to_owned())]);
        let second = BTreeMap::from([("phoenix_project_name".to_owned(), "b".to_owned())]);
        let one = tenants
            .route_for(&default, None, Some(&first), &[])
            .expect("route");
        let two = tenants
            .route_for(&default, None, Some(&second), &[])
            .expect("route");
        assert_eq!(built.builds(), 2);
        assert!(!Arc::ptr_eq(
            one.provider.as_ref().expect("one"),
            two.provider.as_ref().expect("two")
        ));
    }
}

mod transport {
    use std::collections::BTreeMap;

    use betterlite_logging::otel::config::ExporterOwner;

    use crate::otel::support::{Built, cache, creds, default_tracer, settings};

    #[test]
    fn transport_keeps_credentials_off_a_headerless_exporter() {
        let built = Built::default();
        let tenants = cache(
            settings("in_memory", Some(ExporterOwner::ArizeAx), None, None),
            Some("arize"),
            &built,
        );
        let default = default_tracer();
        let route = tenants
            .route_for(&default, Some(&creds("S")), None, &[])
            .expect("route");
        // nowhere to stamp them, so the span stays on the default backend rather than the console
        assert!(route.provider.is_none());
        assert!(!route.detached);
        assert_eq!(built.builds(), 0);
    }

    #[tokio::test]
    async fn transport_routes_credentials_over_grpc() {
        let built = Built::default();
        let tenants = cache(
            settings(
                "otlp_grpc",
                Some(ExporterOwner::ArizeAx),
                None,
                Some("http://localhost:4317"),
            ),
            Some("arize"),
            &built,
        );
        let default = default_tracer();
        let route = tenants
            .route_for(&default, Some(&creds("S")), None, &[])
            .expect("route");
        assert!(route.detached);
        assert!(route.provider.is_some());
    }

    #[tokio::test]
    async fn transport_keeps_project_headers_off_grpc() {
        let built = Built::default();
        let tenants = cache(
            settings(
                "otlp_grpc",
                Some(ExporterOwner::ArizePhoenix),
                None,
                Some("http://localhost:4317"),
            ),
            Some("arize_phoenix"),
            &built,
        );
        let default = default_tracer();
        let auth = BTreeMap::from([("phoenix_project_name".to_owned(), "proj".to_owned())]);
        let route = tenants
            .route_for(&default, None, Some(&auth), &[])
            .expect("route");
        assert!(route.provider.is_none());
        assert!(!route.detached);
        assert_eq!(built.builds(), 0);
    }
}
