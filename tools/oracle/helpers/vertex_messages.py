from collections.abc import Callable
from typing import Final, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.vertex_ai.vertex_ai_partner_models.anthropic.transformation import (
    VertexAIPartnerModelsAnthropicMessagesConfig,
)
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]
BETA_HEADER: Final = "anthropic-beta"
NO_PARAMS: Final[JsonObject] = {}
JSON: Final = TypeAdapter(Json)
STR: Final = TypeAdapter(str)
BETA_VALUES: Final = TypeAdapter(list[Json])
MESSAGES: Final = TypeAdapter(list[JsonObject])
EnvironmentReply: TypeAlias = tuple[JsonObject, str | None]
ValidateEnvironment: TypeAlias = Callable[
    [JsonObject, str, list[Json], JsonObject, JsonObject, str | None, str | None],
    EnvironmentReply,
]
TransformRequest: TypeAlias = Callable[
    [str, list[JsonObject], JsonObject, GenericLiteLLMParams, JsonObject],
    JsonObject,
]


class Input(TypedDict):
    access_token: str
    credential_project: str
    model: str
    messages: list[Json]
    optional_params: JsonObject
    headers: JsonObject
    vertex_project: str | None
    vertex_location: str | None
    api_base: str | None


class Config(Protocol):
    @property
    def validate_anthropic_messages_environment(  # ast-grep-ignore: no-long-names
        self,
    ) -> ValidateEnvironment: ...

    @property
    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
    ) -> TransformRequest: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
    ) -> str: ...


# ast-grep-ignore: no-long-names
class FixedAccess(VertexAIPartnerModelsAnthropicMessagesConfig):
    def __init__(self, access_token: str, project: str) -> None:
        self.fixed_token = access_token
        self.fixed_project = project

    # The fixed token stands in for the service-account file a live deployment loads.
    @override
    def get_access_token(
        self,
        credentials: VERTEX_CREDENTIALS_TYPES | None,
        project_id: str | None,
        _retry_reauth: bool | None = None,
    ) -> tuple[str, str]:
        if credentials is not None or _retry_reauth:
            raise ValueError("the fixed access token has no credentials to refresh")
        if project_id is None:
            return self.fixed_token, self.fixed_project
        return self.fixed_token, project_id


def params_for(argument: Input) -> JsonObject:
    params: JsonObject = {}
    project = argument["vertex_project"]
    if project is not None:
        params["vertex_project"] = project
    location = argument["vertex_location"]
    if location is not None:
        params["vertex_location"] = location
    return params


def beta_tokens(headers: JsonObject) -> list[str]:
    beta = headers.pop(BETA_HEADER, None)
    if beta is None:
        return []
    return sorted(STR.validate_python(beta).split(","))


def url_answer(
    config: Config,
    api_base: str | None,
    model: str,
) -> tuple[str, str | None]:
    url = config.get_complete_url(api_base, None, model, NO_PARAMS, NO_PARAMS)
    missing: str | None = None
    try:
        config.get_complete_url(None, None, model, NO_PARAMS, NO_PARAMS)
    except ValueError as error:
        missing = str(error)
    return url, missing


def answer(argument: Input, config: Config) -> Json:
    headers, api_base = config.validate_anthropic_messages_environment(
        dict(argument["headers"]),
        argument["model"],
        list(argument["messages"]),
        dict(argument["optional_params"]),
        params_for(argument),
        None,
        argument["api_base"],
    )
    tokens = beta_tokens(headers)
    body = config.transform_anthropic_messages_request(
        argument["model"],
        MESSAGES.validate_python(argument["messages"]),
        dict(argument["optional_params"]),
        GenericLiteLLMParams(),
        NO_PARAMS,
    )
    url, missing = url_answer(config, api_base, argument["model"])
    result: JsonObject = {
        "headers": headers,
        "beta": BETA_VALUES.validate_python(tokens),
        "url": url,
        "missing_base": missing,
        "body": body,
    }
    return JSON.validate_python(result)


def run(argument: Input) -> Json:
    # an unknown model otherwise prints its banner to stdout, which carries the answer
    litellm.suppress_debug_info = True
    config: Config = FixedAccess(
        argument["access_token"],
        argument["credential_project"],
    )
    return answer(argument, config)
