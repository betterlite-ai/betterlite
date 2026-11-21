import asyncio
from typing import NotRequired, TypedDict
from urllib.parse import parse_qs, urlparse

from litellm.proxy._experimental.mcp_server import byok_oauth_endpoints
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

LOOPBACK_REDIRECT = "http://127.0.0.1:1456/callback"
SESSION_USER = "byok-pkce-user"


class Input(TypedDict):
    code_verifier: str
    code_challenge: str


class TokenBody(TypedDict):
    access_token: NotRequired[str]
    token_type: NotRequired[str]
    expires_in: NotRequired[int]
    error: NotRequired[str]


TOKEN_BODY = TypeAdapter(TokenBody)


def request() -> Request:
    scope: Scope = {"type": "http", "path": "/v1/mcp/oauth/token"}
    return Request(scope=scope)


async def store_code(code_challenge: str) -> str:
    redirect = await byok_oauth_endpoints.byok_authorize_post(
        request(),
        client_id="",
        redirect_uri=LOOPBACK_REDIRECT,
        code_challenge=code_challenge,
        code_challenge_method="S256",
        state="",
        server_id="",
        api_key="sk-oracle",
        user_api_key_dict=UserAPIKeyAuth(user_id=SESSION_USER),
    )
    location = redirect.headers["location"]
    return parse_qs(urlparse(location).query)["code"][0]


async def redeem(code: str, code_verifier: str) -> TokenBody:
    response = await byok_oauth_endpoints.byok_token(
        request(),
        grant_type="authorization_code",
        code=code,
        redirect_uri="",
        code_verifier=code_verifier,
        client_id="",
    )
    return TOKEN_BODY.validate_json(response.body)


def run(input_value: Input) -> bool:
    code = asyncio.run(store_code(input_value["code_challenge"]))
    body = asyncio.run(redeem(code, input_value["code_verifier"]))
    # a wrong PKCE answer is invalid_grant; every later failure means the PKCE check passed
    return body.get("error") != "invalid_grant"
