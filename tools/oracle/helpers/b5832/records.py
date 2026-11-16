from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.fireworks_ai.completion.transformation import FireworksAITextCompletionConfig
from litellm.llms.fireworks_ai.responses.transformation import FireworksAIResponsesAPIConfig
from litellm.types.llms.openai import OpenAITextCompletionUserMessage as UserMessage
from litellm.types.llms.openai import ResponseInputParam
from litellm.types.responses.main import DeleteResponseResult
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from pydantic import JsonValue, TypeAdapter

MODEL: Final = "accounts/fireworks/models/test-model"
API_ORIGIN: Final = "https://api.fireworks.ai"
FIXED_START: Final = datetime(2024, 1, 1, tzinfo=UTC)
CALL_ID: Final = "00000000000040008000000000000001"
# an unknown model makes LiteLLM print to stdout, corrupting the helper JSON
litellm.suppress_debug_info = True


class CatalogModel(TypedDict):
    key: str
    model_info: dict[str, JsonValue]


class TextMessage(TypedDict):
    role: Literal["user"]
    content: str | list[str] | list[int] | list[list[int]] | None


class EnvValues(TypedDict, total=False):
    fireworks_api_key: str | None
    fireworks_ai_api_key: str | None
    fireworksai_api_key: str | None
    fireworks_ai_token: str | None
    api_base: str | None


class SupportedStep(TypedDict):
    op: Literal["supported"]


class CapabilityStep(TypedDict):
    op: Literal["capability"]


class MapParamsStep(TypedDict):
    op: Literal["map_params"]
    non_default_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]


class ExtraBodyStep(TypedDict):
    op: Literal["extra_body"]
    model: str
    optional_params: NotRequired[dict[str, JsonValue]]


class CompletionStep(TypedDict):
    op: Literal["completion"]
    model: str
    messages: list[TextMessage]
    optional_params: NotRequired[dict[str, JsonValue]]


class HeadersStep(TypedDict):
    op: Literal["headers"]
    headers: dict[str, str]
    session: dict[str, JsonValue]
    api_key: NotRequired[str | None]
    env: NotRequired[EnvValues]


class UrlStep(TypedDict):
    op: Literal["url"]
    api_base: NotRequired[str | None]
    env: NotRequired[EnvValues]


class TransformStep(TypedDict):
    op: Literal["transform"]
    model: str
    input: JsonValue
    optional_params: NotRequired[dict[str, JsonValue]]


class DeleteStep(TypedDict):
    op: Literal["delete"]
    path: str


Step = (
    SupportedStep
    | CapabilityStep
    | MapParamsStep
    | ExtraBodyStep
    | CompletionStep
    | HeadersStep
    | UrlStep
    | TransformStep
    | DeleteStep
)


class DeletionResult(TypedDict):
    id: str | None
    object: str | None
    deleted: bool | None


class Input(TypedDict):
    catalog: NotRequired[list[CatalogModel]]
    steps: list[Step]


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
DELETED: Final[TypeAdapter[DeletionResult]] = TypeAdapter(DeletionResult)


class TextCompletionConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def map_extra_body_params(
        self, optional_params: dict[str, JsonValue], model: str
    ) -> dict[str, JsonValue]: ...

    def transform_text_completion_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[UserMessage],
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...


class ResponsesConfig(Protocol):
    @property
    def custom_llm_provider(self) -> LlmProviders: ...

    def supports_native_websocket(self) -> bool: ...

    def supports_native_file_search(self) -> bool: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        litellm_params: GenericLiteLLMParams | None,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_responses_api_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        input_value: str | ResponseInputParam,
        params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def transform_delete_response_api_response(  # ast-grep-ignore: no-long-names
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> DeleteResponseResult: ...


COMPLETION: TextCompletionConfig = FireworksAITextCompletionConfig()
CONFIG: ResponsesConfig = FireworksAIResponsesAPIConfig()


def model_cost_of(parsed: Input) -> dict[str, dict[str, JsonValue]]:
    entries = parsed.get("catalog")
    if entries is None:
        return {}
    return {entry["key"]: entry["model_info"] for entry in entries}


def optional_params_of(
    step: MapParamsStep | ExtraBodyStep | CompletionStep | TransformStep,
) -> dict[str, JsonValue]:
    optional: dict[str, JsonValue] = {}
    given = step.get("optional_params")
    if given is not None:
        optional.update(given)
    return optional
