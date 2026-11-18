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
    pub router_name: String,
    pub config: Config,
    pub index: Index,
    pub scorer: Scorer,
    // unknown configuration keys LiteLLM accepts and ignores
    pub warnings: Vec<String>,
}

impl QualityRouter {
    pub fn new(
        router_name: &str,
        configured: Option<&section::RouterConfig>,
        default_model: Option<&str>,
        deployments: &[section::Deployment],
    ) -> Result<Self, QualityRouterError> {
        let mut config = Config::resolve(configured);
        // __init__: the caller's default_model, resolved from quality_router_default_model, wins
        if let Some(explicit_model) = default_model {
            config.default_model = Some(explicit_model.to_owned());
        }
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
        let Some(present) = messages.filter(|declared| !declared.is_empty()) else {
            return Ok(None);
        };
        let text = RequestText::extract(present)?;
        let Some(user_message) = text.user_message.as_deref() else {
            let Some(model) = self.config.default_model.as_deref() else {
                return Err(QualityRouterError::NoUserMessage);
            };
            let decision = Decision::DefaultFallback {
                router_model_name: self.router_name.clone(),
                routed_model: model.to_owned(),
            };
            return Ok(Some(Self::finish(present, model, decision, metadata)?));
        };
        if let Some((model, keyword, quality_tier)) = self.index.keyword_match(user_message) {
            let decision = Decision::Keyword {
                router_model_name: self.router_name.clone(),
                routed_model: model.to_owned(),
                matched_keyword: keyword.to_owned(),
                quality_tier,
            };
            return Ok(Some(Self::finish(present, model, decision, metadata)?));
        }
        let (model, decision) = self.tier_route(&text, user_message)?;
        Ok(Some(Self::finish(present, &model, decision, metadata)?))
    }

    // the classifier and the tier index pick the model, with the decision the record and the stash read
    fn tier_route(
        &self,
        text: &RequestText,
        user_message: &str,
    ) -> Result<(String, Decision), QualityRouterError> {
        let outcome = self.scorer.score_and_classify(&PromptInput {
            prompt: user_message,
            system_prompt: text.system_prompt.as_deref(),
        })?;
        let quality_tier = self.config.quality_tier(outcome.tier.name())?;
        let model = self
            .index
            .model_for_tier(quality_tier, self.config.default_model.as_deref())?;
        let decision = Decision::QualityTier {
            router_model_name: self.router_name.clone(),
            routed_model: model.to_owned(),
            quality_tier,
            complexity_tier: outcome.tier.name().to_owned(),
            score: outcome.score,
            signals: outcome.signals,
        };
        Ok((model.to_owned(), decision))
    }

    fn finish<'messages>(
        messages: &'messages [Message],
        model: &str,
        decision: Decision,
        metadata: &mut Option<Json>,
    ) -> Result<Route<'messages>, QualityRouterError> {
        if let Some(document) = decision.stash_document()? {
            decision::stash(metadata, document);
        }
        Ok(Route {
            model: model.to_owned(),
            messages,
            decision,
        })
    }
}
