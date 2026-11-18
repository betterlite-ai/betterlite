use betterlite_http::user_url::validate::Resolve;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::bedrock::flags::MessageCachePoints;
use betterlite_prompts::bedrock::projection::{project_message, project_messages};
use betterlite_prompts::bedrock_images::remote::RemoteImage;
use betterlite_prompts::bedrock_processor::conversation::{Conversion, converse_messages};
use betterlite_prompts::bedrock_tools::capabilities::CachePointSupport;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::message_content::Message;

use crate::converse_messages::TransformedMessages;
use crate::converse_params::wire::transcode;
use crate::converse_request::error::ConverseRequestError;
use crate::converse_request::{ConverseMedia, ConverseRequestInput};

const USER_CONTINUE_FIELD: &str = "user_continue_message";

pub(super) struct MessagesRequest<'request, R, Ids> {
    pub(super) model: &'request str,
    pub(super) messages: &'request [Message],
    pub(super) user_continue_message: Option<Json>,
    pub(super) cache_points: CachePointSupport,
    pub(super) modify_params: bool,
    pub(super) remote: &'request RemoteImage<R>,
    pub(super) new_id: &'request Ids,
}

// B181's converse conversion with the model, the continue message and the caller's tool names.
pub(super) async fn convert<R, Ids>(
    request: &MessagesRequest<'_, R, Ids>,
    names: &mut ToolNameMap,
) -> Result<Vec<Json>, ConverseRequestError>
where
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let user_continue_message = request
        .user_continue_message
        .as_ref()
        .map(|value| transcode::<Message, _>(value).map(|message| project_message(&message)))
        .transpose()?;
    let (projected, _assistant_continue) = project_messages(request.messages, None, names);
    let cache_points = MessageCachePoints {
        support: request.cache_points,
    };
    let conversion = Conversion {
        model: Some(request.model),
        cache_points: &cache_points,
        remote: request.remote,
        user_continue_message: user_continue_message.as_ref(),
        assistant_continue_message: None,
        modify_params: request.modify_params,
    };
    Ok(converse_messages(&projected, &conversion, request.new_id).await?)
}

// The continue message leaves litellm_params once and joins the conversion; the name map keeps every rename.
pub(super) async fn convert_messages<Lookup, R, Ids>(
    input: &mut ConverseRequestInput<'_, Lookup>,
    transformed: &TransformedMessages,
    names: &mut ToolNameMap,
    cache_points: CachePointSupport,
    media: &ConverseMedia<'_, R, Ids>,
) -> Result<Vec<Json>, ConverseRequestError>
where
    Lookup: Sync,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let user_continue_message = input
        .litellm_params
        .as_mut()
        .and_then(|params| params.shift_remove(USER_CONTINUE_FIELD))
        .filter(|value| !matches!(value, Json::Null));
    convert(
        &MessagesRequest {
            model: input.model,
            messages: &transformed.messages,
            user_continue_message,
            cache_points,
            modify_params: input.settings.messages.modify_params,
            remote: media.remote,
            new_id: media.new_id,
        },
        names,
    )
    .await
}
