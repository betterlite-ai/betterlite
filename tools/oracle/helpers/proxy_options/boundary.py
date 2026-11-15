from collections.abc import Callable
from typing import Protocol, TypeAlias, runtime_checkable

from litellm.llms.litellm_proxy.chat.transformation import LiteLLMProxyChatConfig
from litellm.llms.litellm_proxy.image_edit.transformation import LiteLLMProxyImageEditConfig
from litellm.llms.litellm_proxy.image_generation.transformation import (
    LiteLLMProxyImageGenerationConfig,
)
from litellm.llms.litellm_proxy.responses.transformation import LiteLLMProxyResponsesAPIConfig
from litellm.types.router import LiteLLM_Params
from litellm.types.utils import LlmProviders
from pydantic import ConfigDict, JsonValue, TypeAdapter

TransformRequest: TypeAlias = Callable[
    [str, list[JsonValue], dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
    dict[str, JsonValue],
]
GenerationEnvironment: TypeAlias = Callable[
    [
        dict[str, str],
        str,
        list[JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
        str | None,
    ],
    dict[str, str],
]
GenerationUrl: TypeAlias = Callable[
    [str | None, str | None, str, dict[str, JsonValue], dict[str, JsonValue], bool | None],
    str,
]
EditEnvironment: TypeAlias = Callable[
    [dict[str, str], str, str | None, dict[str, JsonValue] | None, str | None],
    dict[str, str],
]
EditUrl: TypeAlias = Callable[[str, str | None, dict[str, JsonValue]], str]


@runtime_checkable
class ChatConfig(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def get_models(self, api_key: str | None = None, api_base: str | None = None) -> list[str]: ...

    def get_api_key(self, api_key: str | None = None) -> str | None: ...

    def should_use_litellm_proxy_by_default(  # ast-grep-ignore: no-long-names
        self, litellm_params: LiteLLM_Params | None = None
    ) -> bool: ...

    def litellm_proxy_get_custom_llm_provider_info(  # ast-grep-ignore: no-long-names
        self, model: str, api_base: str | None = None, api_key: str | None = None
    ) -> tuple[str, str, str | None, str | None]: ...


@runtime_checkable
class ResponsesConfig(Protocol):
    @property
    def custom_llm_provider(self) -> LlmProviders: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def supports_native_websocket(self) -> bool: ...


@runtime_checkable
class GenerationConfig(Protocol):
    @property
    def validate_environment(self) -> GenerationEnvironment: ...

    @property
    def get_complete_url(self) -> GenerationUrl: ...


@runtime_checkable
class EditConfig(Protocol):
    @property
    def validate_environment(self) -> EditEnvironment: ...

    @property
    def get_complete_url(self) -> EditUrl: ...


ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)

CHAT: ChatConfig = TypeAdapter(ChatConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyChatConfig()
)
RESPONSES: ResponsesConfig = TypeAdapter(ResponsesConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyResponsesAPIConfig()
)
GENERATION: GenerationConfig = TypeAdapter(
    GenerationConfig, config=ARBITRARY
).validate_python(LiteLLMProxyImageGenerationConfig())
EDIT: EditConfig = TypeAdapter(EditConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyImageEditConfig()
)
