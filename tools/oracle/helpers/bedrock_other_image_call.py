import asyncio
import os
from collections.abc import Callable, Coroutine
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from functools import partial
from typing import Literal, NotRequired, TypedDict, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1593
import httpx
from aiohttp import ClientSession
from litellm.exceptions import Timeout
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.image_generation import image_handler
from litellm.llms.bedrock.image_generation.image_handler import BedrockImagePreparedRequest
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ImageResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

Client = HTTPHandler | AsyncHTTPHandler | None
Opts = dict[str, Json]
Hdrs = dict[str, str] | None
Text = str | None
Generate = Callable[
    [str, str, ImageResponse, Opts, Logging, float | None, bool, Text, Hdrs, Client, Text],
    ImageResponse | Coroutine[None, None, ImageResponse],
]
Prepare = Callable[[str, Opts, Text, Hdrs, Logging, str, Text], BedrockImagePreparedRequest]


class Input(TypedDict):
    function: str
    probes: list[dict[str, Json]]


@dataclass
class Environment:
    region_name: str | None = None
    region: str | None = None
    session_region: str | None = None
    runtime_endpoint: str | None = None


@dataclass
class Reply:
    status: int = 200
    headers: dict[str, str] = field(default_factory=dict)
    body: str = "{}"
    failure: Literal["timeout", "failed"] | None = None


@dataclass
class Probe:
    model: str = "amazon.titan-image-generator-v2:0"
    prompt: str = "a cat"
    optional_params: dict[str, Json] = field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    api_base: str | None = None
    api_key: str | None = None
    configured_bearer_token: str | None = None
    environment: Environment = field(default_factory=Environment)
    now: int = 1700000000
    reply: Reply = field(default_factory=Reply)
    supplied: Literal["none", "sync", "async"] = "sync"
    aimg_generation: bool = False
    timeout_ms: int | None = None
    created: int = 42


class Images(TypedDict):
    created: int
    data: list[dict[str, Json]]


class Args(TypedDict):
    complete_input_dict: dict[str, Json]
    api_base: NotRequired[str]


@dataclass
class LoggedCall:
    additional_args: Args
    input: Json = None
    original_response: Json = None


PROBE = TypeAdapter(Probe)
DATA = TypeAdapter(dict[str, Json])
BODY = TypeAdapter(Json)
LOGGED = TypeAdapter(LoggedCall)
TIMEOUT = TypeAdapter(float | None)
IMAGES = TypeAdapter(Images)


class FakeLogging(list[tuple[str, dict[str, Json]]]):
    def pre_call(self, **params: Json) -> None:
        self.append(("pre_call", params))

    def post_call(self, **params: Json) -> None:
        self.append(("post_call", params))

    def logged(self, name: str) -> LoggedCall | None:
        for call, params in self:
            if call == name:
                return LOGGED.validate_python(params)
        return None


@dataclass
class Fixture:
    probe: Probe
    supplied: list[tuple[str, str]] = field(default_factory=list)
    factory: list[tuple[str, str]] = field(default_factory=list)
    factory_calls: list[tuple[str, int | None]] = field(default_factory=list)
    logging: FakeLogging = field(default_factory=FakeLogging)

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
        botocore.auth.get_current_datetime = partial(datetime.fromtimestamp, self.probe.now, tz=UTC)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1612
        image_handler._get_httpx_client = self.sync_client  # type: ignore[attr-defined]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L112
        image_handler.get_async_httpx_client = self.async_client  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L157

    @property
    def logger(self) -> Logging:
        return self.logging  # type: ignore[return-value]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L82

    @property
    def used(self) -> list[tuple[str, str]]:
        return self.factory or self.supplied

    def respond(self, recorder: list[tuple[str, str]], request: httpx.Request) -> httpx.Response:
        recorder.append((str(request.url), request.content.decode("utf-8")))
        reply = self.probe.reply
        if reply.failure == "timeout":
            raise httpx.ReadTimeout("read timed out")
        if reply.failure == "failed":
            raise httpx.ReadError("the transport failed")
        return httpx.Response(reply.status, headers=reply.headers, content=reply.body.encode())

    def sync_client(self, params: dict[str, Json] | None = None) -> HTTPHandler:  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L112
        self.factory_calls.append(("sync", None))
        return HTTPHandler(
            client=httpx.Client(transport=httpx.MockTransport(partial(self.respond, self.factory)))
        )

    def async_client(
        self,
        llm_provider: str,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L157
        params: dict[str, Json] | None = None,
        shared_session: ClientSession | None = None,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L157
    ) -> AsyncHTTPHandler:
        seconds = None if params is None else TIMEOUT.validate_python(params["timeout"])
        milliseconds = None if seconds is None else round(seconds * 1000)
        self.factory_calls.append(("async", milliseconds))
        return AsyncHTTPHandler(transport=httpx.MockTransport(partial(self.respond, self.factory)))


def generation(fixture: Fixture) -> ImageResponse:
    probe = fixture.probe
    transport = httpx.MockTransport(partial(fixture.respond, fixture.supplied))
    clients: dict[str, Client] = {
        "sync": HTTPHandler(client=httpx.Client(transport=transport)),
        "async": AsyncHTTPHandler(transport=transport),
        "none": None,
    }
    generate = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L72
        Generate, image_handler.BedrockImageGeneration().image_generation
    )
    pending = generate(
        probe.model,
        probe.prompt,
        ImageResponse(created=probe.created),
        probe.optional_params,
        fixture.logger,
        None if probe.timeout_ms is None else probe.timeout_ms / 1000,
        probe.aimg_generation,
        probe.api_base,
        probe.extra_headers,
        clients[probe.supplied],
        probe.api_key,
    )
    if probe.aimg_generation:
        return asyncio.run(pending)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L90
    return pending  # type: ignore[return-value]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L90


class Handler(image_handler.BedrockImageGeneration):
    def prepare(self, p: Probe, logger: Logging) -> BedrockImagePreparedRequest:
        prepare = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L205
            Prepare, self._prepare_request
        )
        return prepare(
            p.model, p.optional_params, p.api_base, p.extra_headers, logger, p.prompt, p.api_key
        )


def prepare_value(probe: Probe) -> dict[str, Json]:
    fixture = Fixture(probe)
    prepared = Handler().prepare(probe, fixture.logger)
    pre_call = LOGGED.validate_python(fixture.logging[0][1])
    return {
        "endpoint_url": prepared.endpoint_url,
        "prepared_url": prepared.prepped.url,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L267
        "headers": dict(prepared.prepped.headers),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L267
        "body": prepared.body.decode("utf-8"),
        "data": DATA.validate_python(prepared.data),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_generation/image_handler.py#L49
        "pre_call": {
            "input": pre_call.input,
            "api_base": pre_call.additional_args.get("api_base"),
            "complete_input_dict": pre_call.additional_args["complete_input_dict"],
        },
    }


def call_value(probe: Probe) -> Json:
    fixture = Fixture(probe)
    result = generation(fixture)
    post_call = fixture.logging.logged("post_call")
    logged: Json = None
    if post_call is not None:
        logged = {
            "input": post_call.input,
            "original_response": post_call.original_response,
            "complete_input_dict": post_call.additional_args["complete_input_dict"],
        }
    dumped: dict[str, Json] = result.model_dump()
    images = IMAGES.validate_python(dumped)
    data: list[Json] = [{"b64_json": image["b64_json"]} for image in images["data"]]
    request = fixture.used[0]
    return {
        "created": images["created"],
        "data": data,
        "posted_url": request[0],
        "posted_body": BODY.validate_json(request[1]),
        "post_call": logged,
    }


def selection_value(probe: Probe) -> Json:
    fixture = Fixture(probe)
    generation(fixture)
    calls = fixture.factory_calls
    factory, timeout = calls[-1] if calls else ("none", None)
    url = fixture.used[0][0]
    return {
        "factory": factory,
        "timeout_ms": timeout,
        "supplied_requests": len(fixture.supplied),
        "factory_requests": len(fixture.factory),
        "posted_url": url,
    }


def runtime_value(probe: Probe) -> Json:
    value = prepare_value(replace(probe, optional_params=dict(probe.optional_params)))
    fixture = Fixture(probe)
    generation(fixture)
    return {**value, "posted_url": fixture.used[0][0]}


def error_value(probe: Probe) -> Json:
    fixture = Fixture(probe)
    try:
        generation(fixture)
    except (BedrockError, Timeout) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L861
        status: int | None = error.status_code
    except (httpx.HTTPError, TypeError, ValueError):
        status = None
    else:
        return {"ok": True, "status": None, "post_call": True}
    logged = fixture.logging.logged("post_call")
    return {"ok": False, "status": status, "post_call": logged is not None}


def run(input_value: Input) -> Json:
    handlers = {
        "image_family_dispatch": call_value,
        "image_runtime_url": runtime_value,
        "signed_json_body": prepare_value,
        "extra_headers_outside_body": prepare_value,
        "base64_image_result": call_value,
        "image_http_error": error_value,
        "image_transport_selection": selection_value,
    }
    return [
        handlers[input_value["function"]](PROBE.validate_python(p)) for p in input_value["probes"]
    ]
