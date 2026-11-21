from typing import Literal, TypedDict

from litellm.proxy._experimental.mcp_server.discoverable_endpoints import (
    _append_query_params,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/discoverable_endpoints.py#L518-L522
    _validate_token_response,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/discoverable_endpoints.py#L568-L615
    client_supplied_redirect_uris,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException


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


def rejection(error: HTTPException) -> dict[str, JsonValue]:
    validated = Rejection.model_validate(error, from_attributes=True)
    return {
        "status": validated.status_code,
        "detail": validated.detail,
    }


def token_rules(value: TokenRules) -> JsonValue:
    try:
        _validate_token_response(value["token_response"], value["rules"], value["server_id"])
    except HTTPException as error:
        return rejection(error)
    return None


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "client_supplied_redirect_uris":
            answer = TypeAdapter(JsonValue).validate_python(
                client_supplied_redirect_uris(input_value["value"])
            )
        case "validate_token_response":
            answer = token_rules(input_value)
        case "append_query_params":
            answer = _append_query_params(input_value["url"], input_value["params"])
    return answer
