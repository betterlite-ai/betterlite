use std::future::Future;

use betterlite_anthropic::skills::wire::DeleteSkillResponse;

use super::handler::{SkillHttpRequest, SkillsHandlerError, SkillsHttpHandler};
use super::hosted::{HostedSkillRequest, HostedSkillsHandler};
use super::service::SkillAnswer;

impl SkillAnswer for DeleteSkillResponse {
    fn http_answer<Http>(
        http: &Http,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Self, SkillsHandlerError>> + Send
    where
        Http: SkillsHttpHandler + ?Sized,
    {
        http.delete_skill(request)
    }

    fn hosted_answer<Hosted>(
        hosted: &Hosted,
        request: HostedSkillRequest<'_>,
    ) -> impl Future<Output = Result<Self, SkillsHandlerError>> + Send
    where
        Hosted: HostedSkillsHandler + ?Sized,
    {
        hosted.delete_skill(request)
    }
}
