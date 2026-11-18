from typing import NotRequired, TypeAlias

from pydantic import JsonValue
from typing_extensions import TypedDict

Params: TypeAlias = dict[str, JsonValue]


class MinterSpec(TypedDict):
    token: str
    error: NotRequired[str | None]


class RequestScenario(TypedDict):
    environment: dict[str, str]
    query: str | list[str]
    optional_params: NotRequired[Params]


class RequestInput(TypedDict):
    scenarios: list[RequestScenario]


class HeaderScenario(TypedDict):
    environment: dict[str, str]
    headers: dict[str, str]
    minter: MinterSpec
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class HeaderInput(TypedDict):
    scenarios: list[HeaderScenario]


class ReplyScenario(TypedDict):
    environment: dict[str, str]
    body: str
    status: NotRequired[int | None]
    headers: NotRequired[dict[str, str]]
    optional_params: NotRequired[Params | None]


class ReplyInput(TypedDict):
    scenarios: list[ReplyScenario]


class CapScenario(TypedDict):
    environment: dict[str, str]
    query: str | list[str]
    body: str
    optional_params: NotRequired[Params]
    status: NotRequired[int | None]


class MaxResultsInput(TypedDict):
    scenarios: list[CapScenario]


class UrlScenario(TypedDict):
    environment: dict[str, str]
    api_base: NotRequired[str | None]


class SettingsInput(TypedDict):
    urls: list[UrlScenario]
    requests: list[RequestScenario]


class Input(TypedDict, total=False):
    request: RequestInput
    headers: HeaderInput
    citations: ReplyInput
    max_results: MaxResultsInput
    cap: MaxResultsInput
    reply: ReplyInput
    error: ReplyInput
    settings: SettingsInput
    price: ReplyInput
