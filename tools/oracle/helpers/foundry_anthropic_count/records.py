from typing import Literal, NotRequired, TypeAlias, TypedDict

from pydantic import JsonValue as Json, TypeAdapter

DECODER: TypeAdapter[Json] = TypeAdapter(Json)

Env: TypeAlias = dict[str, str]
Params: TypeAlias = dict[str, Json]
Messages: TypeAlias = list[dict[str, object]]


class Reply(TypedDict):
    status: NotRequired[int]
    body: str


class Globals(TypedDict):
    api_key: NotRequired[str | None]
    azure_key: NotRequired[str | None]


class HandleCase(TypedDict):
    model: str
    messages: Messages
    api_key: str
    api_base: str
    system: NotRequired[Json]
    tools: NotRequired[Messages]
    timeout: NotRequired[float]
    request_timeout: NotRequired[float]
    litellm_params: NotRequired[Params]
    globals: NotRequired[Globals]
    environment: NotRequired[Env]
    reply: NotRequired[Reply]


class CounterCase(TypedDict):
    model: str
    messages: NotRequired[Messages | None]
    contents: NotRequired[Messages | None]
    system: NotRequired[Json]
    tools: NotRequired[Messages]
    request_timeout: NotRequired[float]
    litellm_params: NotRequired[Params]
    globals: NotRequired[Globals]
    environment: NotRequired[Env]
    provider: NotRequired[str]
    request_model: NotRequired[str]
    reply: NotRequired[Reply]


class EndpointCall(TypedDict):
    function: Literal["endpoint"]
    api_bases: list[str]


class HandleCall(TypedDict):
    function: Literal["handle"]
    cases: list[HandleCase]


class CounterCall(TypedDict):
    function: Literal["counter"]
    cases: list[CounterCase]


Input: TypeAlias = EndpointCall | HandleCall | CounterCall
