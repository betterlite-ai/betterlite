import time
from collections.abc import Iterator
from datetime import UTC, datetime
from functools import partial
from typing import Protocol, runtime_checkable

import httpx
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.fireworks_ai.chat.transformation import FireworksAIConfig
from litellm.types.utils import CallTypes, ModelResponse, ModelResponseStream
from pydantic import ConfigDict, TypeAdapter

from .wire import FIXED_TIME, FIXED_UUID, MODEL, Object

ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class ChunkHandler(Protocol):
    def chunk_parser(self, chunk: Object) -> ModelResponseStream: ...


@runtime_checkable
class ChatConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        _model: str,
        _messages: list[Object],
        _optional_params: Object,
        litellm_params: Object,
        api_key: str | None,
        _api_base: str | None,
        /,
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
        optional_params: Object,
        _litellm_params: Object,
        _encoding: None,
        /,
    ) -> ModelResponse: ...

    def get_api_key(self, *, api_key: str | None = None) -> str | None: ...

    def get_models(self, api_key: str | None = None, api_base: str | None = None) -> list[str]: ...

    def get_model_response_iterator(
        self,
        streaming_response: Iterator[str],
        *,
        sync_stream: bool,
        json_mode: bool,
    ) -> ChunkHandler: ...


CHAT: ChatConfig = TypeAdapter(ChatConfig, config=ARBITRARY).validate_python(FireworksAIConfig())


class FixedUuid:
    def __call__(self) -> UUID:
        return litellm_uuid.UUID(FIXED_UUID)


def pin_clock() -> None:
    # LiteLLM mints ids and timestamps with fastuuid and time, and the cases fix both
    litellm_uuid.uuid4 = FixedUuid()
    time.time = partial(float, FIXED_TIME)


def logging_object() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type=CallTypes.completion,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b6072",
        function_id="b6072",
    )
