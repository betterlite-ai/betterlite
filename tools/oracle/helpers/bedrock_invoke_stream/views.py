import httpx
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.bedrock.chat.invoke_transformations.base_invoke_transformation import (
    AmazonInvokeConfig,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue

from .wire import JSON, LoggingRecorder, MockReply, Recorder


# B004 keeps the model it was built with, the provider it was told and the headers it was handed.
class WrapperView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    model: str
    custom_llm_provider: str
    response_headers: httpx.Headers = Field(alias="_response_headers")


def wrapper_payload(wrapper: CustomStreamWrapper) -> JsonValue:
    view = WrapperView.model_validate(wrapper)
    pairs = sorted([name, value] for name, value in view.response_headers.items())
    payload: dict[str, JsonValue] = {
        "model": view.model,
        "custom_llm_provider": view.custom_llm_provider,
        "response_headers": JSON.validate_python(pairs),
    }
    return JSON.validate_python(payload)


def config_payload() -> JsonValue:
    config = AmazonInvokeConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
    payload: dict[str, JsonValue] = {
        "has_custom_stream_wrapper": config.has_custom_stream_wrapper,
        "supports_stream_param_in_request_body": config.supports_stream_param_in_request_body,
    }
    return JSON.validate_python(payload)


def transport_payload(recorder: Recorder) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "arguments": JSON.validate_python(recorder.calls[0]),
        "client": recorder.client,
        "config": config_payload(),
    }
    return JSON.validate_python(payload)


def identity_payload(
    wrapper: CustomStreamWrapper, logging: LoggingRecorder, reply: MockReply
) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "wrapper": wrapper_payload(wrapper),
        "config": config_payload(),
        "log": JSON.validate_python(logging.calls[0]),
        "post": JSON.validate_python(reply.posts[0]),
    }
    return JSON.validate_python(payload)
