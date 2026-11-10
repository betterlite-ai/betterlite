use betterlite_anthropic::skills::wire::DeleteSkillResponse;

use super::handler::SkillsHttpHandler;
use super::hosted::HostedSkillsHandler;
use super::service::{AnswerFuture, SkillAnswer, SkillChannel};

impl SkillAnswer for DeleteSkillResponse {
    fn answer<Http, Hosted>(channel: SkillChannel<'_, Http, Hosted>) -> AnswerFuture<'_, Self>
    where
        Http: SkillsHttpHandler + ?Sized,
        Hosted: HostedSkillsHandler + ?Sized,
    {
        match channel {
            SkillChannel::Http { http, request } => Box::pin(http.delete_skill(request)),
            SkillChannel::Hosted { hosted, request } => Box::pin(hosted.delete_skill(request)),
        }
    }
}
