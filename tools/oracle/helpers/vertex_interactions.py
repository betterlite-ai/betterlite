from collections.abc import Callable, Mapping
from dataclasses import dataclass
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.vertex_ai.interactions.transformation import VertexAIInteractionsConfig
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
HeadersBuilder: TypeAlias = Callable[[Mapping[str, str], str, GenericLiteLLMParams], dict[str, str]]
LifecycleBuilder: TypeAlias = Callable[
    [str, str, GenericLiteLLMParams, Mapping[str, str]], tuple[str, Object]
]
NO_HEADERS: Final[Mapping[str, str]] = {}
EMPTY_PARAMS: Final[Mapping[str, str]] = {}
EMPTY_HEADERS: Final[Mapping[str, str]] = {}


class Params(TypedDict, total=False):
    vertex_project: str
    vertex_ai_project: str
    vertex_location: str
    vertex_ai_location: str


class MintFields(TypedDict):
    token: str
    project: NotRequired[str]


class UrlCall(TypedDict):
    function: Literal["url"]
    litellm_params: NotRequired[Params]
    api_base: NotRequired[str]
    stream: NotRequired[bool]


class LifecycleCall(TypedDict):
    function: Literal["lifecycle"]
    action: Literal["get", "delete", "cancel"]
    interaction_id: str
    litellm_params: NotRequired[Params]
    api_base: NotRequired[str]


class HeadersCall(TypedDict):
    function: Literal["headers"]
    litellm_params: NotRequired[Params]
    headers: NotRequired[dict[str, str]]


class IdentityCall(TypedDict):
    function: Literal["identity"]


Call = UrlCall | LifecycleCall | HeadersCall | IdentityCall


class Input(TypedDict):
    mint: MintFields
    calls: list[Call]


class InteractionView(TypedDict):
    headers: HeadersBuilder
    get: LifecycleBuilder
    delete: LifecycleBuilder
    cancel: LifecycleBuilder


@dataclass(frozen=True)
class MintFixture:
    token: str
    project: str | None

    def __call__(
        self, credentials: VERTEX_CREDENTIALS_TYPES | None, project_id: str | None
    ) -> tuple[str, str]:
        del credentials
        resolved = self.project if self.project is not None else project_id
        return (self.token, resolved if resolved is not None else "")


def params_of(call: UrlCall | LifecycleCall | HeadersCall) -> GenericLiteLLMParams:
    given = call.get("litellm_params")
    return GenericLiteLLMParams().model_validate(given if given is not None else EMPTY_PARAMS)


def url_reply(config: VertexAIInteractionsConfig, call: UrlCall) -> JsonValue:
    given = call.get("litellm_params")
    params = given if given is not None else EMPTY_PARAMS
    url = config.get_complete_url(call.get("api_base"), None, None, params, call.get("stream"))
    return {"url": url}


def lifecycle_reply(view: InteractionView, call: LifecycleCall) -> JsonValue:
    api_base = call.get("api_base")
    base = api_base if api_base is not None else ""
    builder = view[call["action"]]
    url, body = builder(call["interaction_id"], base, params_of(call), NO_HEADERS)
    return {"url": url, "body": body}


def headers_reply(view: InteractionView, call: HeadersCall) -> JsonValue:
    given = call.get("headers")
    caller = given if given is not None else EMPTY_HEADERS
    headers = view["headers"](caller, "interaction", params_of(call))
    return {"headers": dict[str, JsonValue](headers)}


def resolve(config: VertexAIInteractionsConfig, view: InteractionView, call: Call) -> JsonValue:
    if call["function"] == "url":
        return url_reply(config, call)
    if call["function"] == "lifecycle":
        return lifecycle_reply(view, call)
    if call["function"] == "headers":
        return headers_reply(view, call)
    return {
        "provider": config.custom_llm_provider.value,
        "api_version": config.api_version,
        "default_location": config.get_default_vertex_location(),
    }


def run(input_value: Input) -> JsonValue:
    mint = input_value["mint"]
    config = VertexAIInteractionsConfig(
        MintFixture(token=mint["token"], project=mint.get("project"))
    )
    view: InteractionView = cast(InteractionView, {"headers": config.validate_environment, "get": config.transform_get_interaction_request, "delete": config.transform_delete_interaction_request, "cancel": config.transform_cancel_interaction_request})  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/interactions/transformation.py#L87-L149
    return {"calls": [resolve(config, view, call) for call in input_value["calls"]]}
