use std::future::Future;

use betterlite_anthropic::skills::wire::Skill;

use super::handler::{SkillHttpRequest, SkillsHandlerError, SkillsHttpHandler};
use super::hosted::{HostedSkillRequest, HostedSkillsHandler};
use super::service::SkillAnswer;

impl SkillAnswer for Skill {
    fn http_answer<Http>(
        http: &Http,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Self, SkillsHandlerError>> + Send
    where
        Http: SkillsHttpHandler + ?Sized,
    {
        http.get_skill(request)
    }

    fn hosted_answer<Hosted>(
        hosted: &Hosted,
        request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<Self, SkillsHandlerError>> + Send
    where
        Hosted: HostedSkillsHandler + ?Sized,
    {
        hosted.get_skill(request)
    }
}
