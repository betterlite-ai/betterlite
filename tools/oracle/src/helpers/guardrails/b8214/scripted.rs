use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::builder::SemanticRouterBuilder;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::route::Route;
use betterlite_guardrails::semantic_guard_hook::scoring::{RouteChoice, SemanticRouteScorer};
use tokio::sync::Mutex;

// the embedding service is the only step a case cannot run, so the case scripts which route the router answers
pub(super) struct ScriptedRouter {
    route_name: Option<String>,
    matches: Arc<Mutex<bool>>,
    scored: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SemanticRouteScorer for ScriptedRouter {
    type Error = Infallible;

    async fn score(&self, text: &str) -> Result<Vec<RouteChoice>, Self::Error> {
        self.scored.lock().await.push(text.to_owned());
        let matched = *self.matches.lock().await;
        let choice = if matched {
            RouteChoice {
                name: self.route_name.clone(),
                similarity_score: Some(1.0_f64),
            }
        } else {
            RouteChoice {
                name: None,
                similarity_score: None,
            }
        };
        Ok(vec![choice])
    }
}

pub(super) struct ScriptedBuilder {
    pub(super) matches: Arc<Mutex<bool>>,
    pub(super) scored: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SemanticRouterBuilder for ScriptedBuilder {
    type Router = ScriptedRouter;

    type Error = Infallible;

    async fn build(
        &self,
        routes: Vec<Route>,
        _global_threshold: f64,
    ) -> Result<Self::Router, Self::Error> {
        Ok(ScriptedRouter {
            route_name: routes.first().map(|route| route.name.clone()),
            matches: Arc::clone(&self.matches),
            scored: Arc::clone(&self.scored),
        })
    }
}
