from collections.abc import Callable, Iterator, Mapping
from typing import Literal, Protocol

from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: Mapping[str, JsonValue] | None


class HandlerView(Protocol):
    json_mode: bool | None


class LitellmGlobals(Protocol):
    model_cost: dict[str, JsonValue]
    open_ai_chat_completion_models: set[str]
    open_ai_text_completion_models: set[str]
    supports_reasoning: Callable[[str, str | None], bool]


class ChatApi(Protocol):
    @property
    def custom_llm_provider(self) -> str: ...

    @property
    def _aws_signer(self) -> BaseAWSLLM: ...

    def get_config(self) -> dict[str, JsonValue]: ...

    def _get_openai_compatible_provider_info(  # ast-grep-ignore: no-long-names  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L58-L84
        self,
        api_base: str | None,
        api_key: str | None,
        litellm_params: GenericLiteLLMParams | None = None,
        model: str | None = None,
    ) -> tuple[str | None, str | None]: ...

    def validate_environment(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L86-L108
        self,
        headers: Mapping[str, JsonValue],
        model: str,
        messages: list[JsonValue],
        optional_params: Mapping[str, JsonValue],
        litellm_params: Mapping[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, JsonValue]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: Mapping[str, str]
    ) -> ErrorView: ...

    def get_model_response_iterator(
        self,
        streaming_response: Iterator[JsonValue],
        *,
        sync_stream: bool,
        json_mode: bool | None = False,
    ) -> HandlerView: ...

    def sign_request(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/common_utils.py#L78-L127
        self,
        headers: Mapping[str, str | None],
        optional_params: Mapping[str, JsonValue],
        request_data: Mapping[str, JsonValue],
        api_base: str,
        *,
        api_key: str | None = None,
        model: str | None = None,
        stream: bool | None = None,
        fake_stream: bool | None = None,
    ) -> tuple[dict[str, str | None], bytes | None]: ...


class MessagesApi(Protocol):
    @property
    def custom_llm_provider(self) -> str: ...

    def uses_get_llm_provider_api_base(self) -> bool: ...

    def get_complete_url(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/messages/transformation.py#L58-L67
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Mapping[str, JsonValue],
        litellm_params: Mapping[str, JsonValue],
        *,
        stream: bool | None = None,
    ) -> str: ...


class PassthroughApi(Protocol):
    def _get_aws_region_name(
        self,
        optional_params: Mapping[str, JsonValue],
        model: str | None = None,
        model_id: str | None = None,
    ) -> str: ...

    def get_runtime_endpoint(
        self,
        api_base: str | None,
        aws_bedrock_runtime_endpoint: str | None,
        aws_region_name: str,
        endpoint_type: Literal["runtime", "agent", "agentcore"] | None = "runtime",
    ) -> tuple[str, str]: ...

    def get_bedrock_bearer_token(
        self, litellm_params: Mapping[str, JsonValue]
    ) -> str | None: ...
