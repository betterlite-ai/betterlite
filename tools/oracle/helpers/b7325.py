import os
from typing import Literal, TypeAlias
from unittest import mock

import httpx
from litellm.llms.gigachat.authenticator import (
    _parse_token_response,  # pyright: ignore[reportPrivateUsage]
    get_access_token,
)
from pydantic import BaseModel, ConfigDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    access_token: str | None = None
    credentials: str | None = None
    scope: str | None = None
    auth_url: str | None = None


class AccessTokenInput(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: Literal["access_token"]
    request: TokenRequest = TokenRequest()
    environment: dict[str, str] = {}


class ReplyInput(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: Literal["reply"]
    body: str


def access_token(supplied: AccessTokenInput) -> Json:
    params: dict[str, object] = {}
    if supplied.request.access_token is not None:
        params["gigachat_access_token"] = supplied.request.access_token
    if supplied.request.scope is not None:
        params["gigachat_scope"] = supplied.request.scope
    if supplied.request.auth_url is not None:
        params["gigachat_auth_url"] = supplied.request.auth_url
    with mock.patch.dict(os.environ, supplied.environment):
        token = get_access_token(
            credentials=supplied.request.credentials,
            scope=None,
            auth_url=None,
            litellm_params=params,
        )
    return {"token": token}


def reply(supplied: ReplyInput) -> Json:
    response = httpx.Response(
        200,
        content=supplied.body.encode(),
        request=httpx.Request("POST", "http://127.0.0.1:9/api/v2/oauth"),
    )
    token, expires_at = _parse_token_response(response)
    return {"token": token, "expires_at": expires_at}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if operation == "access_token":
        return access_token(AccessTokenInput.model_validate(input_value))
    if operation == "reply":
        return reply(ReplyInput.model_validate(input_value))
    raise ValueError(f"unknown b7325 operation: {operation!r}")
