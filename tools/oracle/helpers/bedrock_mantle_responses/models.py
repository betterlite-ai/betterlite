from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict

from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]

ENVIRONMENT_KEYS: Final = (
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_REGION",
    "AWS_REGION_NAME",
    "BEDROCK_MANTLE_API_BASE",
    "BEDROCK_MANTLE_API_KEY",
    "BEDROCK_MANTLE_REGION",
)


class MantleCalls(Protocol):
    def get_complete_url(self, api_base: str | None, litellm_params: Object) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, JsonValue],
        model: str,
        litellm_params: GenericLiteLLMParams | None,
    ) -> dict[str, JsonValue]: ...


class UrlCase(TypedDict):
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    use_openai_path: NotRequired[bool]
    environment: NotRequired[dict[str, str]]


class MapCase(TypedDict):
    model: str
    params: NotRequired[dict[str, JsonValue]]
    drop_params: NotRequired[bool]
    use_openai_path: NotRequired[bool]


class ErrorProbe(TypedDict):
    status: int
    message: str
    headers: NotRequired[dict[str, str]]


class AuthCase(TypedDict):
    api_key: NotRequired[str]
    project_id: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    error: NotRequired[ErrorProbe]
    environment: NotRequired[dict[str, str]]


class HoistCase(TypedDict):
    model: str
    input: JsonValue
    params: NotRequired[dict[str, JsonValue]]


class UrlPayload(TypedDict):
    cases: list[UrlCase]


class MapPayload(TypedDict):
    cases: list[MapCase]


class AuthPayload(TypedDict):
    cases: list[AuthCase]


class HoistPayload(TypedDict):
    cases: list[HoistCase]


URL_PAYLOAD: Final = TypeAdapter(UrlPayload)
MAP_PAYLOAD: Final = TypeAdapter(MapPayload)
AUTH_PAYLOAD: Final = TypeAdapter(AuthPayload)
HOIST_PAYLOAD: Final = TypeAdapter(HoistPayload)
