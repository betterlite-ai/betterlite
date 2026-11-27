import asyncio
import os
from collections.abc import Coroutine
from dataclasses import dataclass, field
from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
from litellm.exceptions import Timeout
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.image_edit.handler import BedrockImageEdit
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
HANDLER: Final[type[BedrockImageEdit]] = BedrockImageEdit
# what httpx adds itself and what the signature covers never enter the shared answer
OMITTED_HEADERS: Final = frozenset(
    {
        "accept",
        "accept-encoding",
        "authorization",
        "connection",
        "host",
        "user-agent",
        "x-amz-date",
        "x-amz-security-token",
    }
)
Client = HTTPHandler | AsyncHTTPHandler | None
JSON: Final = TypeAdapter(JsonValue)
DATA: Final = TypeAdapter(dict[str, JsonValue])


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


class Input(TypedDict):
    function: str
    model_cost: dict[str, JsonValue]
    probes: list[dict[str, JsonValue]]


HIDDEN: Final = TypeAdapter(Hidden)


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


PROBE: Final = TypeAdapter(Probe)


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


class Recorder(Logging):
    def __init__(self) -> None:
        self.pre: dict[str, JsonValue] | None = None
        self.post: dict[str, JsonValue] | None = None

    @override
    def pre_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        match kwargs["additional_args"]:
            case {"api_base": api_base, "complete_input_dict": body}:
                self.pre = {
                    "input": kwargs["input"],
                    "api_base": api_base,
                    "complete_input_dict": body,
                }

    @override
    def post_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        match kwargs["additional_args"]:
            case {"complete_input_dict": body}:
                self.post = {
                    "input": kwargs["input"],
                    "original_response": kwargs["original_response"],
                    "complete_input_dict": body,
                }


class EditHandler(Protocol):
    def image_edit(
        self,
        _model: str,
        _image: list[str],
        _prompt: str | None,
        _model_response: ImageResponse,
        _optional_params: dict[str, JsonValue],
        _logging: Logging,
        _timeout: float | None,
        _aimage_edit: bool,
        _api_base: str | None,
        _extra_headers: dict[str, str] | None,
        _client: Client,
        _api_key: str | None,
        /,
    ) -> ImageResponse | Coroutine[None, None, ImageResponse]: ...


def client_for(probe: Probe, fixture: Fixture) -> HTTPHandler | AsyncHTTPHandler:
    transport = httpx.MockTransport(fixture.respond)
    match probe.supplied:
        case "sync":
            return HTTPHandler(client=httpx.Client(transport=transport))
        case "async":
            return AsyncHTTPHandler(transport=transport)
        case "none":
            raise ValueError("the oracle case supplies a client")


def perform(probe: Probe, logging: Logging) -> tuple[Fixture, ImageResponse]:
    fixture = Fixture(probe)
    handler: EditHandler = HANDLER()
    timeout = probe.timeout_ms
    pending = handler.image_edit(
        probe.model,
        probe.images,
        probe.prompt,
        ImageResponse(),
        probe.optional_params,
        logging,
        None if timeout is None else timeout / 1000,
        probe.aimage_edit,
        probe.api_base,
        probe.extra_headers,
        client_for(probe, fixture),
        probe.api_key,
    )
    match pending:
        case Coroutine():
            return fixture, asyncio.run(pending)
        case object():
            return fixture, pending


def shown_headers(request: httpx.Request) -> dict[str, str]:
    kept = (
        (name, value)
        for name, value in request.headers.items()
        if name not in OMITTED_HEADERS
    )
    return dict(kept)


def image_entries(response: ImageResponse) -> list[JsonValue]:
    images = response.data
    if images is None:
        return []
    return [
        {"b64_json": image.b64_json} for image in images if image.b64_json is not None
    ]


def cost_of(response: ImageResponse) -> JsonValue:
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return hidden["additional_headers"].get(COST_HEADER)


def prepare_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    fixture, _ = perform(probe, logging)
    request = fixture.requests[0]
    return {
        "endpoint_url": str(request.url),
        "headers": JSON.validate_python(shown_headers(request)),
        "body": request.content.decode("utf-8"),
        "data": DATA.validate_python(JSON.validate_json(request.content)),
        "remaining_params": JSON.validate_python(sorted(probe.optional_params)),
        "pre_call": logging.pre,
    }


def result_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    try:
        _, response = perform(probe, logging)
    except (BedrockError, Timeout) as error:
        return {
            "ok": False,
            "status": error.status_code,
            "post_call": logging.post is not None,
        }
    except (httpx.HTTPError, TypeError, ValueError):
        return {"ok": False, "status": None, "post_call": logging.post is not None}
    return {
        "ok": True,
        "images": image_entries(response),
        "cost": cost_of(response),
        "post_call": logging.post,
    }


def error_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    try:
        perform(probe, logging)
    except (BedrockError, Timeout) as error:
        status: int | None = error.status_code
    except (httpx.HTTPError, TypeError, ValueError):
        status = None
    else:
        return {"ok": True, "status": None, "post_call": True}
    return {"ok": False, "status": status, "post_call": logging.post is not None}


def model_error_answer(probe: Probe) -> JsonValue:
    try:
        perform(probe, Recorder())
    except ValueError as error:
        return {"ok": False, "status": None, "message": str(error)}
    else:
        return {"ok": True, "status": None, "message": None}


def selection_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    fixture, _ = perform(probe, logging)
    return {
        "factory": "none",
        "timeout_ms": None,
        "supplied_requests": len(fixture.requests),
        "factory_requests": 0,
        "posted_url": str(fixture.requests[0].url),
    }


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    handlers = {
        "prepare": prepare_answer,
        "result": result_answer,
        "error": error_answer,
        "model_error": model_error_answer,
        "selection": selection_answer,
    }
    return [
        handlers[input_value["function"]](PROBE.validate_python(probe))
        for probe in input_value["probes"]
    ]
