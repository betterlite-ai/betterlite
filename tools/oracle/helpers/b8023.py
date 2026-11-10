import os
from collections.abc import Callable
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.linkup.search.transformation import LinkupSearchConfig
from litellm.llms.searxng.search.transformation import SearXNGSearchConfig
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = JsonValue

Transform: TypeAlias = Callable[[str | list[str], dict[str, Json]], dict[str, Json]]
Complete: TypeAlias = Callable[[str | None, dict[str, Json], dict[str, Json]], str]
Validate: TypeAlias = Callable[..., dict[str, str]]
ResponseTransform: TypeAlias = Callable[[httpx.Response, object], BaseModel]


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")
    query: str | list[str]
    optional_params: dict[str, Json]


class BodyInput(BaseModel):
    model_config = ConfigDict(extra="forbid")
    requests: list[Request]


class EnvironmentInput(BaseModel):
    model_config = ConfigDict(extra="forbid")
    headers: dict[str, str]
    environment: dict[str, str]
    api_key: str | None
    api_base: str | None


class UrlInput(BaseModel):
    model_config = ConfigDict(extra="forbid")
    environment: dict[str, str]
    api_base: str | None
    requests: list[Request]


class ReplyInput(BaseModel):
    model_config = ConfigDict(extra="forbid")
    body: str


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: Literal[
        "linkup_body",
        "linkup_environment",
        "linkup_reply",
        "linkup_url",
        "searxng_body",
        "searxng_environment",
        "searxng_reply",
        "searxng_url",
    ]
    params: dict[str, Json]


def split_url(url: str) -> Json:
    address, separator, query = url.partition("?")
    params = sorted(query.split("&")) if separator else []
    return cast(Json, {"address": address, "params": params})


def transform_of(config: LinkupSearchConfig | SearXNGSearchConfig) -> Transform:
    return cast(Transform, config.transform_search_request)


def complete_of(config: LinkupSearchConfig | SearXNGSearchConfig) -> Complete:
    return cast(Complete, config.get_complete_url)


def validate_of(config: LinkupSearchConfig | SearXNGSearchConfig) -> Validate:
    return cast(Validate, config.validate_environment)


def response_json(config: LinkupSearchConfig | SearXNGSearchConfig, body: str) -> Json:
    request = httpx.Request("POST", "https://search.test")
    response = httpx.Response(200, content=body.encode(), request=request)
    transform = cast(
        ResponseTransform,
        config.transform_search_response,  # pyright: ignore[reportUnknownMemberType]
    )
    return cast(Json, transform(response, None).model_dump())


def linkup_body(params: dict[str, Json]) -> Json:
    request = BodyInput.model_validate(params)
    transform = transform_of(LinkupSearchConfig())
    return {"bodies": [transform(item.query, item.optional_params) for item in request.requests]}


def searxng_body(params: dict[str, Json]) -> Json:
    request = BodyInput.model_validate(params)
    transform = transform_of(SearXNGSearchConfig())
    return {"bodies": [transform(item.query, item.optional_params) for item in request.requests]}


def linkup_url(params: dict[str, Json]) -> Json:
    request = UrlInput.model_validate(params)
    config = LinkupSearchConfig()
    transform = transform_of(config)
    complete = complete_of(config)
    with patch.dict(os.environ, request.environment, clear=True):
        urls: list[Json] = []
        for item in request.requests:
            body = transform(item.query, item.optional_params)
            urls.append(split_url(complete(request.api_base, item.optional_params, body)))
    return {"urls": urls}


def searxng_url(params: dict[str, Json]) -> Json:
    request = UrlInput.model_validate(params)
    config = SearXNGSearchConfig()
    transform = transform_of(config)
    complete = complete_of(config)
    with patch.dict(os.environ, request.environment, clear=True):
        urls: list[Json] = []
        for item in request.requests:
            body = transform(item.query, item.optional_params)
            urls.append(split_url(complete(request.api_base, item.optional_params, body)))
    return {"urls": urls}


def linkup_environment(params: dict[str, Json]) -> Json:
    request = EnvironmentInput.model_validate(params)
    validate = validate_of(LinkupSearchConfig())
    with patch.dict(os.environ, request.environment, clear=True):
        headers = validate(
            dict(request.headers), api_key=request.api_key, api_base=request.api_base
        )
    return cast(Json, {"headers": headers})


def searxng_environment(params: dict[str, Json]) -> Json:
    request = EnvironmentInput.model_validate(params)
    validate = validate_of(SearXNGSearchConfig())
    with patch.dict(os.environ, request.environment, clear=True):
        headers = validate(
            dict(request.headers), api_key=request.api_key, api_base=request.api_base
        )
    return cast(Json, {"headers": headers})


def linkup_reply(params: dict[str, Json]) -> Json:
    request = ReplyInput.model_validate(params)
    return {"response": response_json(LinkupSearchConfig(), request.body)}


def searxng_reply(params: dict[str, Json]) -> Json:
    request = ReplyInput.model_validate(params)
    return {"response": response_json(SearXNGSearchConfig(), request.body)}


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "linkup_body": linkup_body,
    "linkup_environment": linkup_environment,
    "linkup_reply": linkup_reply,
    "linkup_url": linkup_url,
    "searxng_body": searxng_body,
    "searxng_environment": searxng_environment,
    "searxng_reply": searxng_reply,
    "searxng_url": searxng_url,
}


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    _ = litellm
    return HANDLERS[supplied.operation](supplied.params)
