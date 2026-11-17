use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_media_dispatch::call::{
    CharacterCreateInput, CharacterGetInput, VideoEditCall, VideoEditInput, VideoExtensionInput,
};

use super::video::required;
use super::{OTHER_OUTCOME, VideoCase, VideoOp, failure};

pub(super) fn extended_call(
    case: &VideoCase,
    reply: Vec<u8>,
) -> Result<VideoEditCall, Box<HelperError>> {
    match case.op {
        VideoOp::Edit => Ok(VideoEditCall::Edit(Box::new(VideoEditInput {
            prompt: required(case.prompt.as_deref(), "prompt")?.to_owned(),
            video_id: required(case.video_id.as_deref(), "video_id")?.to_owned(),
            video_file: case
                .video_file
                .as_ref()
                .map(|video| video.as_bytes().to_vec()),
            extra_body: case.extra_body.clone(),
            reply,
        }))),
        VideoOp::Extension => Ok(VideoEditCall::Extension(Box::new(VideoExtensionInput {
            prompt: required(case.prompt.as_deref(), "prompt")?.to_owned(),
            video_id: required(case.video_id.as_deref(), "video_id")?.to_owned(),
            seconds: required(case.seconds.as_deref(), "seconds")?.to_owned(),
            extra_body: case.extra_body.clone(),
            reply,
        }))),
        VideoOp::CreateCharacter => Ok(VideoEditCall::CreateCharacter(Box::new(
            CharacterCreateInput {
                name: required(case.name.as_deref(), "name")?.to_owned(),
                video: required(case.video.as_deref(), "video")?
                    .as_bytes()
                    .to_vec(),
                reply,
            },
        ))),
        VideoOp::GetCharacter => Ok(VideoEditCall::GetCharacter(Box::new(CharacterGetInput {
            character_id: required(case.character_id.as_deref(), "character_id")?.to_owned(),
            reply,
        }))),
        VideoOp::List | VideoOp::Status | VideoOp::Delete | VideoOp::Content | VideoOp::Remix => {
            Err(failure("Variant", OTHER_OUTCOME))
        }
    }
}
