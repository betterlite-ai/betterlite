import asyncio
import os
from typing import Final, Literal, NotRequired, TypedDict
from urllib.parse import parse_qsl

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.proxy import proxy_server
from litellm.proxy.auth.oauth2_check import Oauth2Handler
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

STATUS_PREFIX: Final = "Oauth 2.0 Token validation failed"
ENDPOINT_ABSENT: Final = "OAUTH_TOKEN_INFO_ENDPOINT environment variable is not set"


class Upstream(TypedDict):
    status: int
    body: str
    failure: NotRequired[Literal["network"]]


class Call(TypedDict):
    token: str
    endpoint: NotRequired[str | None]
    client_id: NotRequired[str | None]
    client_secret: NotRequired[str | None]
    user_id_field_name: NotRequired[str | None]
    user_role_field_name: NotRequired[str | None]
    user_team_id_field_name: NotRequired[str | None]
    response: Upstream


class Identity(TypedDict):
    user_id: str | None
    user_role: str | None
    team_id: str | None


class RequestView(TypedDict):
    method: str
    url: str
    authorization: str | None
    content_type: str | None
    form: list[tuple[str, str]]


class Answer(TypedDict):
    request: RequestView | None
    identity: Identity | None
    error: dict[str, str] | None


IDENTITY: Final[TypeAdapter[Identity]] = TypeAdapter(Identity)


class Recorder:
    def __init__(self, upstream: Upstream) -> None:
        self.upstream: Final = upstream
        self.request: RequestView | None = None

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.request = RequestView(
            method=request.method,
            url=str(request.url),
            authorization=request.headers.get("authorization"),
            content_type=request.headers.get("content-type"),
            form=parse_qsl(request.content.decode("utf-8"), keep_blank_values=True),
        )
        if self.upstream.get("failure") == "network":
            raise httpx.ConnectError("the identity provider is unreachable", request=request)
        return httpx.Response(self.upstream["status"], content=self.upstream["body"].encode("utf-8"))


class ClientCache(LLMClientCache):
    def __init__(self, client: httpx.AsyncClient) -> None:
        super().__init__()
        self.client: Final = client

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> httpx.AsyncClient:
        del kwargs
        return self.client


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def error_category(error: ValueError) -> str:
    message = str(error)
    if message == ENDPOINT_ABSENT:
        return "endpoint_absent"
    if message.startswith(STATUS_PREFIX):
        return "status"
    return "validation"


def run(input_value: Call) -> Answer:
    set_env("OAUTH_TOKEN_INFO_ENDPOINT", input_value.get("endpoint"))
    set_env("OAUTH_CLIENT_ID", input_value.get("client_id"))
    set_env("OAUTH_CLIENT_SECRET", input_value.get("client_secret"))
    set_env("OAUTH_USER_ID_FIELD_NAME", input_value.get("user_id_field_name"))
    set_env("OAUTH_USER_ROLE_FIELD_NAME", input_value.get("user_role_field_name"))
    set_env("OAUTH_USER_TEAM_ID_FIELD_NAME", input_value.get("user_team_id_field_name"))
    recorder = Recorder(input_value["response"])
    client = httpx.AsyncClient(transport=httpx.MockTransport(recorder.respond))
    # the recorded no-paid-tier deviation enables the MIT premium_user gate the source method reads
    proxy_server.premium_user = True
    litellm.in_memory_llm_clients_cache = ClientCache(client)
    try:
        granted = asyncio.run(Oauth2Handler.check_oauth2_token(token=input_value["token"]))
    except ValueError as error:
        return Answer(
            request=recorder.request,
            identity=None,
            error={
                "class": type(error).__name__,
                "category": error_category(error),
                "message": str(error),
            },
        )
    return Answer(
        request=recorder.request,
        identity=IDENTITY.validate_json(
            granted.model_dump_json(include={"user_id", "user_role", "team_id"})
        ),
        error=None,
    )
