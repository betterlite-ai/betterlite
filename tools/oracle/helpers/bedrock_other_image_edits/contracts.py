import os
from dataclasses import dataclass, field
from typing import Final, Literal, TypedDict

import httpx
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    function: str
    model_cost: dict[str, JsonValue]
    probes: list[dict[str, JsonValue]]


@dataclass
class Environment:
    region_name: str | None = None
    region: str | None = None
    session_region: str | None = None
    runtime_endpoint: str | None = None


@dataclass
class Credentials:
    access_key_id: str | None = None
    secret_access_key: str | None = None
    session_token: str | None = None


@dataclass
class Reply:
    status: int = 200
    headers: dict[str, str] = field(default_factory=dict)
    body: str = '{"images": ["eA=="]}'
    failure: Literal["timeout", "failed"] | None = None


@dataclass
class Probe:
    model: str
    prompt: str | None = None
    images: list[str] = field(default_factory=list)
    optional_params: dict[str, JsonValue] = field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    api_base: str | None = None
    api_key: str | None = None
    configured_bearer_token: str | None = None
    environment: Environment = field(default_factory=Environment)
    credentials: Credentials = field(default_factory=Credentials)
    reply: Reply = field(default_factory=Reply)
    supplied: Literal["none", "sync", "async"] = "sync"
    aimage_edit: bool = False
    timeout_ms: int | None = None


PROBE: Final[TypeAdapter[Probe]] = TypeAdapter(Probe)


@dataclass
class Fixture:
    probe: Probe
    requests: list[httpx.Request] = field(default_factory=list)

    def __post_init__(self) -> None:
        environment = self.probe.environment
        os.environ.clear()
        names = {
            "AWS_CONFIG_FILE": os.devnull,
            "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
            "AWS_REGION_NAME": environment.region_name,
            "AWS_REGION": environment.region,
            "AWS_DEFAULT_REGION": environment.session_region,
            "AWS_BEDROCK_RUNTIME_ENDPOINT": environment.runtime_endpoint,
            "AWS_BEARER_TOKEN_BEDROCK": self.probe.configured_bearer_token,
        }
        os.environ.update({name: value for name, value in names.items() if value is not None})

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        match self.probe.reply.failure:
            case "timeout":
                raise httpx.ReadTimeout("read timed out")
            case "failed":
                raise httpx.ReadError("the transport failed")
            case object():
                reply = self.probe.reply
                return httpx.Response(
                    reply.status,
                    headers=reply.headers,
                    content=reply.body.encode(),
                )


Client = HTTPHandler | AsyncHTTPHandler | None


def client_for(probe: Probe, fixture: Fixture) -> HTTPHandler | AsyncHTTPHandler:
    transport = httpx.MockTransport(fixture.respond)
    match probe.supplied:
        case "sync":
            return HTTPHandler(client=httpx.Client(transport=transport))
        case "async":
            return AsyncHTTPHandler(transport=transport)
        case "none":
            raise ValueError("the oracle case supplies a client")
