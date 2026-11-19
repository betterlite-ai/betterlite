from datetime import UTC, datetime

import openai
from litellm.endpoints.speech.speech_to_completion_bridge.handler import (
    speech_to_completion_bridge_handler as speech_bridge_handler,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import CallTypes
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json

from openai_chat import project_failure


class SpeechChatBridgeCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    input: str
    voice: str | None = None
    optional_params: dict[str, Json] = Field(default_factory=dict)
    litellm_params: dict[str, Json] = Field(default_factory=dict)
    headers: dict[str, str] = Field(default_factory=dict)
    custom_llm_provider: str
    api_key: str
    timeout_seconds: int


class SpeechChatBridgeValue(BaseModel):
    status: int
    content_type: str | None
    body: list[int]


class SpeechChatBridgeSuccess(BaseModel):
    value: SpeechChatBridgeValue


class SpeechChatBridgeResult(BaseModel):
    speech_chat_bridge: SpeechChatBridgeSuccess


def logging_object(call: SpeechChatBridgeCall) -> Logging:
    return Logging(
        model=call.model,
        messages=None,
        stream=False,
        call_type=CallTypes.completion,
        start_time=datetime.now(UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )


def project_success(
    content: bytes, status: int, content_type: str | None
) -> SpeechChatBridgeResult:
    return SpeechChatBridgeResult(
        speech_chat_bridge=SpeechChatBridgeSuccess(
            value=SpeechChatBridgeValue(
                status=status,
                content_type=content_type,
                body=list(content),
            )
        )
    )


def run_speech(call: SpeechChatBridgeCall, api_base: str) -> str:
    litellm_params: dict[str, Json] = {
        **call.litellm_params,
        "api_base": api_base,
        "api_key": call.api_key,
        "timeout": call.timeout_seconds,
        "num_retries": 0,
    }
    try:
        response = speech_bridge_handler.speech(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/endpoints/speech/speech_to_completion_bridge/handler.py#L78-L88
            model=call.model,
            input=call.input,
            voice=call.voice,
            optional_params=call.optional_params,
            litellm_params=litellm_params,
            headers=call.headers,
            logging_obj=logging_object(call),
            custom_llm_provider=call.custom_llm_provider,
        )
    except openai.APIError as error:
        return project_failure(error).model_dump_json(
            by_alias=True, exclude_unset=True
        )
    headers = response.response.headers
    return project_success(
        response.content,
        response.response.status_code,
        next(iter(headers.get_list("content-type")), None),
    ).model_dump_json(by_alias=True, exclude_unset=True)


class SpeechChatBridgeRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_chat_bridge: SpeechChatBridgeCall

    def serialize(self, api_base: str) -> str:
        return run_speech(self.speech_chat_bridge, api_base)
