import os
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.tavily.search.transformation import TavilySearchConfig
from litellm.llms.you_com.search.transformation import YouComSearchConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Config: TypeAlias = TavilySearchConfig | YouComSearchConfig

ENVIRONMENT_NAMES = (
    "TAVILY_API_KEY",
    "TAVILY_API_BASE",
    "YOUCOM_API_KEY",
    "YOUCOM_API_BASE",
)


class ValidateEnvironment(TypedDict):
    operation: Literal["validate_environment"]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]


class CompleteUrl(TypedDict):
    operation: Literal["complete_url"]
    api_base: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]


class TransformSearchRequest(TypedDict):
    operation: Literal["transform_search_request"]
    query: str | list[str]
    optional_params: NotRequired[dict[str, Json]]
    environment: NotRequired[dict[str, str]]


class TransformSearchResponse(TypedDict):
    operation: Literal["transform_search_response"]
    status: NotRequired[int]
    headers: NotRequired[dict[str, str]]
    body: Json
    environment: NotRequired[dict[str, str]]


Call: TypeAlias = (
    ValidateEnvironment | CompleteUrl | TransformSearchRequest | TransformSearchResponse
)


class Input(TypedDict):
    provider: Literal["tavily", "you_com"]
    calls: list[Call]


def _apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        _ = os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def _validate_environment(config: Config, call: ValidateEnvironment) -> Json:
    return cast(
        Json,
        config.validate_environment(  # pyright: ignore[reportUnknownMemberType]
            headers=dict(call.get("headers", {})),
            api_key=call.get("api_key"),
            api_base=call.get("api_base"),
        ),
    )


def _complete_url(config: Config, call: CompleteUrl) -> Json:
    return config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
        api_base=call.get("api_base"),
        optional_params={},
    )


def _transform_search_request(config: Config, call: TransformSearchRequest) -> Json:
    return cast(
        Json,
        config.transform_search_request(  # pyright: ignore[reportUnknownMemberType]
            query=call["query"],
            optional_params=dict(call.get("optional_params", {})),
        ),
    )


def _transform_search_response(config: Config, call: TransformSearchResponse) -> Json:
    response = httpx.Response(
        status_code=call.get("status", 200),
        headers=call.get("headers", {}),
        json=call["body"],
    )
    transformed = config.transform_search_response(  # pyright: ignore[reportUnknownMemberType]
        raw_response=response, logging_obj=None  # pyright: ignore[reportArgumentType]
    )
    return cast(Json, transformed.model_dump())


def _execute(config: Config, call: Call) -> Json:
    if call["operation"] == "validate_environment":
        return _validate_environment(config, call)
    if call["operation"] == "complete_url":
        return _complete_url(config, call)
    if call["operation"] == "transform_search_request":
        return _transform_search_request(config, call)
    if call["operation"] == "transform_search_response":
        return _transform_search_response(config, call)
    raise ValueError(f"unsupported operation {call['operation']}")


def run(input_value: Input) -> list[Json]:
    config: Config = (
        TavilySearchConfig()
        if input_value["provider"] == "tavily"
        else YouComSearchConfig()
    )
    outcomes: list[Json] = []
    for call in input_value["calls"]:
        _apply_environment(call.get("environment", {}))
        try:
            value = _execute(config, call)
        except Exception:  # noqa: BLE001 - a case compares only that the call failed
            outcomes.append({"ok": False})
        else:
            outcomes.append({"ok": True, "value": value})
    return outcomes
