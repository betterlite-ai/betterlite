from collections.abc import Iterator
from datetime import UTC, datetime
from typing import Protocol, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.edenai.chat.transformation import EdenAIChatConfig
from litellm.llms.edenai.embedding.transformation import EdenAIEmbeddingConfig
from litellm.llms.edenai.messages.transformation import EdenAIAnthropicMessagesConfig
from litellm.llms.edenai.responses.transformation import EdenAIResponsesAPIConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes, EmbeddingResponse, ModelResponse, ModelResponseStream
from pydantic import ConfigDict, JsonValue, TypeAdapter

from .wire import MODEL, Object

ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class ChunkHandler(Protocol):
    def chunk_parser(self, chunk: Object) -> ModelResponseStream: ...


@runtime_checkable
class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

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

    def get_model_response_iterator(
        self,
        streaming_response: Iterator[str],
        *,
        sync_stream: bool,
        json_mode: bool,
    ) -> ChunkHandler: ...

    def get_models(self, api_key: str | None = None, api_base: str | None = None) -> list[str]: ...


@runtime_checkable
class EmbeddingConfig(Protocol):
    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        _model: str,
        _messages: list[Object],
        _optional_params: Object,
        _litellm_params: Object,
        _api_key: str | None = None,
        _api_base: str | None = None,
        /,
    ) -> Object: ...

    def get_complete_url(
        self,
        api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: Object,
        _litellm_params: Object,
        /,
    ) -> str: ...

    def transform_embedding_request(
        self,
        model: str,
        input_value: JsonValue,
        optional_params: Object,
        _headers: Object,
        /,
    ) -> Object: ...

    def transform_embedding_response(
        self,
        _model: str,
        raw_response: httpx.Response,
        model_response: EmbeddingResponse,
        logging_obj: Logging,
        _api_key: str | None,
        _request_data: Object,
        _optional_params: Object,
        _litellm_params: Object,
        /,
    ) -> EmbeddingResponse: ...


@runtime_checkable
class ResponsesConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        _model: str,
        litellm_params: GenericLiteLLMParams | None,
        /,
    ) -> Object: ...

    def get_complete_url(self, api_base: str | None, _litellm_params: Object, /) -> str: ...

    def transform_response_api_response(  # ast-grep-ignore: no-long-names
        self,
        _model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
        /,
    ) -> ResponsesAPIResponse: ...

    def should_fake_stream(
        self,
        _model: str | None,
        *,
        stream: bool | None,
        _custom_llm_provider: str | None = None,
    ) -> bool: ...

    def supports_native_websocket(self) -> bool: ...


@runtime_checkable
class MessagesConfig(Protocol):
    def validate_anthropic_messages_environment(  # ast-grep-ignore: no-long-names
        self,
        headers: dict[str, str],
        _model: str,
        _messages: list[Object],
        _optional_params: Object,
        _litellm_params: Object,
        _api_key: str | None = None,
        _api_base: str | None = None,
        /,
    ) -> tuple[Object, str | None]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: Object,
        _litellm_params: Object,
        /,
    ) -> str: ...

    def transform_anthropic_messages_response(  # ast-grep-ignore: no-long-names
        self,
        _model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
        /,
    ) -> Object: ...


CHAT: ChatConfig = TypeAdapter(ChatConfig, config=ARBITRARY).validate_python(EdenAIChatConfig())
EMBEDDING: EmbeddingConfig = TypeAdapter(EmbeddingConfig, config=ARBITRARY).validate_python(
    EdenAIEmbeddingConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L69
)
RESPONSES: ResponsesConfig = TypeAdapter(ResponsesConfig, config=ARBITRARY).validate_python(
    EdenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/responses/transformation.py#L31
)
MESSAGES: MessagesConfig = TypeAdapter(MessagesConfig, config=ARBITRARY).validate_python(
    EdenAIAnthropicMessagesConfig()
)
STREAM_LINES: Iterator[str] = iter(("data: [DONE]\n\n",))
HANDLER: ChunkHandler = CHAT.get_model_response_iterator(
    STREAM_LINES, sync_stream=True, json_mode=False
)


def logging_object(call_type: CallTypes) -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type=call_type,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b6100",
        function_id="b6100",
    )


def raw_reply(body: str) -> httpx.Response:
    response = httpx.Response(
        status_code=200,
        stream=httpx.ByteStream(body.encode()),
        request=httpx.Request("POST", "https://api.edenai.run/v3"),
    )
    response.read()
    return response
