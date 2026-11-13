use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_anthropic::skills::wire::{DeleteSkillResponse, Skill};
use betterlite_models::decisions::Json;

use super::handler::SkillsHandlerError;

pub struct HostedSkillRequest<'request> {
    pub skill_id: &'request str,
    pub user_api_key_auth: Option<&'request Json>,
    pub litellm_call_id: Option<&'request str>,
}

pub trait HostedSkillsHandler: Send + Sync {
    fn get_skill(
        &self,
        request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send;

    fn delete_skill(
        &self,
        request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send;
}

// LiteLLM's module-level singleton without the global: the factory is injected and the first hosted call stores it
pub struct HostedSkillsHandlers<Handler> {
    pub create: Box<dyn Fn() -> Arc<Handler> + Send + Sync>,
    pub cached: Mutex<Option<Arc<Handler>>>,
}

#[must_use]
pub fn hosted_handler<Handler>(hosted: &HostedSkillsHandlers<Handler>) -> Arc<Handler>
where
    Handler: HostedSkillsHandler,
{
    let mut cached = match hosted.cached.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(handler) = cached.as_ref() {
        return Arc::clone(handler);
    }
    let handler = (hosted.create)();
    cached.replace(Arc::clone(&handler));
    handler
}
