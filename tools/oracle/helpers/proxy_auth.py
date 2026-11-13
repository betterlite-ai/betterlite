import sys
from functools import partial
from types import SimpleNamespace
from typing import Literal
from unittest.mock import patch

import httpx
import litellm
from litellm.proxy_auth import credentials
from litellm.proxy_auth.credentials import (
    AccessToken,
    AzureADCredential,
    GenericOAuth2Credential,
    ProxyAuthHandler,
)
from pydantic import BaseModel, ConfigDict, JsonValue

AZURE_EXPIRES_ON = 5000
ENDPOINT = "https://idp.invalid/oauth2/issue"


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Input(Strict):
    operation: Literal["azure", "oauth2", "handler"]
    params: dict[str, JsonValue]


class AzureParams(Strict):
    installed: bool
    injected: bool
    scopes: list[str]


class Reply(Strict):
    status: int
    body: dict[str, JsonValue]


class Call(Strict):
    at: int
    scope: str


class Oauth2Params(Strict):
    client_id: str
    client_secret: str
    responses: list[Reply]
    calls: list[Call]


class Grant(Strict):
    token: str
    expires_on: int


class Refusal(Strict):
    refuse: str


class Moment(Strict):
    at: int


class HandlerParams(Strict):
    scope: str
    replies: list[Grant | Refusal]
    calls: list[Moment]


class Named:
    def __init__(self, prefix: str, builds: list[str]) -> None:
        builds.append(prefix)
        self.prefix = prefix

    def get_token(self, *scopes: str) -> AccessToken:
        return AccessToken(token=f"{self.prefix}-{scopes[0]}", expires_on=AZURE_EXPIRES_ON)


class Clock:
    def __init__(self) -> None:
        self.now = 0

    def time(self) -> float:
        return float(self.now)


class Poster:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.forms: list[JsonValue] = []

    def __call__(self, url: str, *, data: dict[str, str]) -> httpx.Response:
        self.forms.append(dict(data))
        reply = self.replies.pop(0) if self.replies else Reply(status=500, body={})
        return httpx.Response(reply.status, json=reply.body, request=httpx.Request("POST", url))


class Queue:
    def __init__(self, replies: list[Grant | Refusal]) -> None:
        self.replies = list(replies)
        self.scopes: list[str] = []

    def get_token(self, scope: str) -> AccessToken:
        self.scopes.append(scope)
        if not self.replies:
            raise RuntimeError("no reply left")
        reply = self.replies.pop(0)
        if isinstance(reply, Refusal):
            raise RuntimeError(reply.refuse)
        return AccessToken(token=reply.token, expires_on=reply.expires_on)


def azure(params: AzureParams) -> JsonValue:
    builds: list[str] = []
    identity = SimpleNamespace(DefaultAzureCredential=partial(Named, "chain", builds))
    modules = {"azure.identity": identity if params.installed else None}
    injected = Named("injected", []) if params.injected else None
    results: list[JsonValue] = []
    with patch.dict(sys.modules, modules):
        credential = AzureADCredential(credential=injected)
        built_before = len(builds)
        for scope in params.scopes:
            try:
                token = credential.get_token(scope)
            except ImportError as error:
                results.append({"error": str(error)})
            else:
                results.append({"token": token.token, "expires_on": token.expires_on})
    return {"chains_built_before": built_before, "chains_built": len(builds), "results": results}


def oauth2(params: Oauth2Params) -> JsonValue:
    poster = Poster(params.responses)
    clock = Clock()
    credential = GenericOAuth2Credential(
        client_id=params.client_id, client_secret=params.client_secret, token_url=ENDPOINT
    )
    results: list[JsonValue] = []
    with patch.object(httpx, "post", poster), patch.object(credentials, "time", clock):
        for call in params.calls:
            clock.now = call.at
            try:
                token = credential.get_token(call.scope)
            except httpx.HTTPStatusError:
                results.append({"error": True})
            else:
                results.append({"token": token.token, "expires_on": token.expires_on})
    return {"results": results, "posts": poster.forms}


def handler(params: HandlerParams) -> JsonValue:
    queue = Queue(params.replies)
    clock = Clock()
    auth = ProxyAuthHandler(credential=queue, scope=params.scope)
    results: list[JsonValue] = []
    with patch.object(credentials, "time", clock):
        for moment in params.calls:
            clock.now = moment.at
            try:
                headers = auth.get_auth_headers()
            except RuntimeError:
                results.append({"error": True})
            else:
                results.append({"headers": dict(headers)})
    scopes: list[JsonValue] = [*queue.scopes]
    return {"results": results, "credential_scopes": scopes}


def run(argument: JsonValue) -> JsonValue:
    supplied = Input.model_validate(argument)
    _ = litellm
    match supplied.operation:
        case "azure":
            return azure(AzureParams.model_validate(supplied.params))
        case "oauth2":
            return oauth2(Oauth2Params.model_validate(supplied.params))
        case "handler":
            return handler(HandlerParams.model_validate(supplied.params))
