pub mod config;
pub mod decision;
pub mod error;
pub mod index;
pub mod messages;

mod tests;

use betterlite_config::sections::complexity_scoring::Config as ScoringInput;
use betterlite_config::sections::quality_router as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;

use crate::complexity_scoring::classify::{PromptInput, Scorer};
use crate::complexity_scoring::config::Config as ScoringConfig;

use self::config::Config;
use self::decision::Decision;
use self::error::QualityRouterError;
use self::index::Index;
use self::messages::RequestText;

// PreRoutingHookResponse: the routed model keeps the request's messages untouched
pub struct Route<'messages> {
    pub model: String,
    pub messages: &'messages [Message],
    pub decision: Decision,
}

pub struct QualityRouter {
    router_name: String,
    config: Config,
    index: Index,
    scorer: Scorer,
    // unknown configuration keys LiteLLM accepts and ignores
    pub warnings: Vec<String>,
}

impl QualityRouter {
    pub fn new(
        router_name: &str,
        configured: Option<&section::RouterConfig>,
        deployments: &[section::Deployment],
    ) -> Result<Self, QualityRouterError> {
        let config = Config::resolve(configured);
        let scorer = Scorer::new(ScoringConfig::resolve(&ScoringInput::default())?)?;
        let mut warnings: Vec<String> = config
            .ignored
            .iter()
            .map(|field| {
                format!(
                    "quality_router: ignoring unknown QualityRouterConfig field '{field}' for router '{router_name}'"
                )
            })
            .collect();
        let index = Index::build(&config, deployments, &mut warnings)?;
        Ok(Self {
            router_name: router_name.to_owned(),
            config,
            index,
            scorer,
            warnings,
        })
    }

    // async_pre_routing_hook: a keyword override short-circuits classification; no route for absent messages
    pub fn pre_route<'messages>(
        &self,
        messages: Option<&'messages [Message]>,
        metadata: &mut Option<Json>,
    ) -> Result<Option<Route<'messages>>, QualityRouterError> {
        let Some(messages) = messages.filter(|messages| !messages.is_empty()) else {
            return Ok(None);
        };
        let text = RequestText::extract(messages)?;
        let Some(user_message) = text.user_message.as_deref() else {
            let Some(model) = self.config.default_model.as_deref() else {
                return Err(QualityRouterError::NoUserMessage);
            };
            let decision = Decision::default_fallback(&self.router_name, model);
            return Ok(Some(Self::finish(messages, model, decision, metadata)));
        };
        if let Some((model, keyword, quality_tier)) = self.index.keyword_match(user_message) {
            let decision = Decision::keyword(&self.router_name, model, keyword, quality_tier);
            return Ok(Some(Self::finish(messages, model, decision, metadata)));
        }
        let outcome = self.scorer.score_and_classify(&PromptInput {
            prompt: user_message,
            system_prompt: text.system_prompt.as_deref(),
        })?;
        let quality_tier = self.config.quality_tier(outcome.tier.name())?;
        let model = self
            .index
            .model_for_tier(quality_tier, self.config.default_model.as_deref())?;
        let decision = Decision::quality(&self.router_name, model, quality_tier, &outcome);
        Ok(Some(Self::finish(messages, model, decision, metadata)))
    }

    fn finish<'messages>(
        messages: &'messages [Message],
        model: &str,
        decision: Decision,
        metadata: &mut Option<Json>,
    ) -> Route<'messages> {
        if let Some(document) = decision.stash_document() {
            decision::stash(metadata, document);
        }
        Route {
            model: model.to_owned(),
            messages,
            decision,
        }
    }
}
