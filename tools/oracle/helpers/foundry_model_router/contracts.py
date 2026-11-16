from collections.abc import Callable
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.azure_model_router.transformation import AzureModelRouterConfig
from litellm.llms.azure_ai.common_utils import AzureFoundryModelInfo
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
Messages: TypeAlias = list[JsonObject]
Headers: TypeAlias = dict[str, str]
VERSION_NAMES: Final[tuple[str, ...]] = ("AZURE_API_VERSION",)


class StripCall(TypedDict):
    model: str


class ResponseCall(TypedDict):
    model: str
    body: str


class ReaderCall(TypedDict):
    model: NotRequired[str | None]
    hidden_params: NotRequired[JsonObject | None]


class CatalogCall(TypedDict):
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class VersionCall(TypedDict):
    api_version: NotRequired[str | None]
    global_api_version: NotRequired[str | None]
    environment: dict[str, str]


class PropertyCall(TypedDict):
    global_api_version: NotRequired[str | None]
    environment: dict[str, str]


class Input(TypedDict):
    strips: NotRequired[list[StripCall]]
    responses: NotRequired[list[ResponseCall]]
    readers: NotRequired[list[ReaderCall]]
    catalog: NotRequired[list[CatalogCall]]
    token_models: NotRequired[list[str | None]]
    versions: NotRequired[list[VersionCall]]
    properties: NotRequired[list[PropertyCall]]
    validate: NotRequired[bool]


class ResponseRecord(TypedDict):
    model: NotRequired[str | None]


# the transform methods take LiteLLM message objects, so the config enters through these callables
@runtime_checkable
class RouterConfig(Protocol):
    transform_request: Callable[[str, Messages, JsonObject, JsonObject, JsonObject], JsonObject]
    transform_response: Callable[
        [
            str,
            httpx.Response,
            ModelResponse,
            Logging,
            JsonObject,
            Messages,
            JsonObject,
            JsonObject,
            None,
            str | None,
        ],
        ModelResponse,
    ]


@runtime_checkable
class ModelInfo(Protocol):
    api_version: str | None
    validate_environment: Callable[
        [Headers, str, Messages, JsonObject, JsonObject, str | None, str | None], Headers
    ]


ARBITRARY: Final[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)
CONFIG: Final[RouterConfig] = TypeAdapter(RouterConfig, config=ARBITRARY).validate_python(
    AzureModelRouterConfig()
)
INFO: Final[ModelInfo] = TypeAdapter(ModelInfo, config=ARBITRARY).validate_python(
    AzureFoundryModelInfo()
)
RESPONSE: Final[TypeAdapter[ResponseRecord]] = TypeAdapter(ResponseRecord)
