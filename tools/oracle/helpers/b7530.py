from typing import Literal, TypedDict

from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.discoverable_endpoints import (
    _append_query_params,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/discoverable_endpoints.py#L518-L522
    _validate_token_response,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/discoverable_endpoints.py#L568-L615
    client_supplied_redirect_uris,
)
from pydantic import BaseModel, JsonValue, TypeAdapter


class RedirectUris(TypedDict):
    function: Literal["client_supplied_redirect_uris"]
    value: JsonValue


class TokenRules(TypedDict):
    function: Literal["validate_token_response"]
    token_response: dict[str, JsonValue]
    rules: dict[str, JsonValue]
    server_id: str


class AppendQueryParams(TypedDict):
    function: Literal["append_query_params"]
    url: str
    params: dict[str, str]


Input = RedirectUris | TokenRules | AppendQueryParams


class Rejection(BaseModel):
    status_code: int
    detail: dict[str, JsonValue]


def rejection(error: HTTPException) -> JsonValue:
    validated = Rejection.model_validate(error, from_attributes=True)
    return TypeAdapter(JsonValue).validate_python({
        "status": validated.status_code,
        "detail": validated.detail,
    })


def redirect_uris(value: RedirectUris) -> JsonValue:
    return TypeAdapter(JsonValue).validate_python(client_supplied_redirect_uris(value["value"]))


def token_rules(value: TokenRules) -> JsonValue:
    try:
        _validate_token_response(value["token_response"], value["rules"], value["server_id"])
    except HTTPException as error:
        return rejection(error)
    return None


def query_params(value: AppendQueryParams) -> JsonValue:
    return _append_query_params(value["url"], value["params"])


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "client_supplied_redirect_uris":
            answer = redirect_uris(input_value)
        case "validate_token_response":
            answer = token_rules(input_value)
        case "append_query_params":
            answer = query_params(input_value)
    return answer
