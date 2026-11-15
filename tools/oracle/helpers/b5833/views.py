import datetime as dt
from typing import Protocol, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.groq.chat.transformation import GroqChatConfig
from litellm.llms.groq.stt.transformation import GroqSTTConfig
from litellm.types.utils import CallTypes, ModelResponse, ModelResponseStream
from pydantic import ConfigDict, TypeAdapter

from .wire import MODEL, Object

ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


@runtime_checkable
class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...

    def transform_request(
        self,
        model: str,
        messages: list[Object],
        optional_params: Object,
        litellm_params: Object,
        headers: Object,
    ) -> Object: ...

    def transform_response(
        self,
        _model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        _request_data: Object,
        _messages: list[Object],
        _optional_params: Object,
        _litellm_params: Object,
        _encoding: None,
        /,
        *,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


@runtime_checkable
class ChunkParser(Protocol):
    def chunk_parser(self, chunk: Object) -> ModelResponseStream: ...


@runtime_checkable
class SttConfig(Protocol):
    def map_openai_params_stt(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...


CHAT: ChatConfig = TypeAdapter(ChatConfig, config=ARBITRARY).validate_python(GroqChatConfig())
STT: SttConfig = TypeAdapter(SttConfig, config=ARBITRARY).validate_python(GroqSTTConfig())


def logging_object() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type=CallTypes.completion,
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="b5833",
        function_id="b5833",
    )
