#![cfg(test)]

mod public_endpoints {
    mod resource_hubs_routes {
        use std::sync::{Arc, Mutex};

        use axum::Router;
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use betterlite_auth::agent_registry::AgentRegistry;
        use betterlite_core::api::claude_plugin_policy::DB_NOT_CONNECTED;
        use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
        use betterlite_server::agent_endpoints::admin_actions::ports::PublicGroupStore;
        use betterlite_server::custom_url::CustomUrls;
        use betterlite_server::public_endpoints::resource_hubs::{
            ResourceHubsState, resource_hubs_routes,
        };
        use tokio::sync::RwLock;
        use tower::ServiceExt as _;

        struct NoGroups(Mutex<Option<Vec<String>>>);

        impl PublicGroupStore for NoGroups {
            fn groups(&self) -> Option<Vec<String>> {
                self.0.lock().expect("the group store").clone()
            }

            fn set_groups(&self, groups: Vec<String>) {
                *self.0.lock().expect("the group store") = Some(groups);
            }
        }

        fn state() -> ResourceHubsState {
            ResourceHubsState {
                agents: Arc::new(AgentRegistry::default()),
                public_agent_groups: Arc::new(NoGroups(Mutex::new(None))),
                mcp: Arc::new(RwLock::new(ServerRegistry::new(
                    RegistrySettings {
                        public_mcp_servers: Vec::new(),
                        public_hub_strict: true,
                        internal_ip_ranges: None,
                    },
                    Vec::new(),
                    Vec::new(),
                ))),
                public_mcp_servers: None,
                plugins: None,
                custom_urls: CustomUrls::default(),
            }
        }

        fn get(path: &str) -> Request<Body> {
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .expect("the request builds")
        }

        async fn body_text(application: Router, path: &str) -> (StatusCode, String) {
            let response = application
                .oneshot(get(path))
                .await
                .expect("the route answers");
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("the body reads");
            let text = String::from_utf8(bytes.to_vec()).expect("the body is UTF-8");
            (status, text)
        }

        #[tokio::test]
        async fn agent_hub_answers_with_an_empty_list_when_the_allowlist_is_none() {
            let (status, text) =
                body_text(resource_hubs_routes(state()), "/public/agent_hub").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(text, "[]");
        }

        #[tokio::test]
        async fn mcp_hub_answers_with_an_empty_list_when_no_server_is_public() {
            let (status, text) = body_text(resource_hubs_routes(state()), "/public/mcp_hub").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(text, "[]");
        }

        #[tokio::test]
        async fn skill_hub_answers_500_without_a_database() {
            let (status, text) =
                body_text(resource_hubs_routes(state()), "/public/skill_hub").await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(text, format!(r#"{{"error":"{DB_NOT_CONNECTED}"}}"#));
        }
    }
}
