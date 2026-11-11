import asyncio
import json
import os
from collections.abc import Callable, Coroutine, Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from datetime import UTC, datetime
from functools import partial
from types import SimpleNamespace
from typing import TYPE_CHECKING, Literal, TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import httpx
from botocore.credentials import Credentials as BotoCredentials
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.image_generation import image_handler
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging

Json: TypeAlias = JsonValue
Failure: TypeAlias = Literal["timeout", "failed"]
Supplied: TypeAlias = Literal["none", "sync", "async"]


class Credentials(BaseModel):
    model_config = ConfigDict(extra="forbid")
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None


class Environment(BaseModel):
    model_config = ConfigDict(extra="forbid")
    region_name: str | None = None
    region: str | None = None
    session_region: str | None = None
    runtime_endpoint: str | None = None


class Reply(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status: int = 200
    headers: dict[str, str] = {}
    body: str = "{}"
    failure: Failure | None = None


class Probe(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str = "amazon.titan-image-generator-v2:0"
    prompt: str = "a cat"
    optional_params: dict[str, Json] = {}
    extra_headers: dict[str, str] | None = None
    api_base: str | None = None
    api_key: str | None = None
    configured_bearer_token: str | None = None
    environment: Environment = Environment()
    credentials: Credentials
    now: int = 1700000000
    reply: Reply = Reply()
    supplied: Supplied = "sync"
    aimg_generation: bool = False
    timeout_ms: int | None = None
    created: int = 42


class Probes(BaseModel):
    model_config = ConfigDict(extra="forbid")
    probes: list[Probe]


class FakeLogging:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Json]]] = []

    def pre_call(self, **params: Json) -> None:
        self.calls.append(("pre_call", dict(params)))

    def post_call(self, **params: Json) -> None:
        self.calls.append(("post_call", dict(params)))

    def post_call_value(self) -> Json:
        for name, params in self.calls:
            if name == "post_call":
                additional: dict[str, Json] = cast(
                    "dict[str, Json]", params.get("additional_args", {})
                )
                return {
                    "input": params.get("input"),
                    "original_response": params.get("original_response"),
                    "complete_input_dict": additional.get("complete_input_dict"),
                }
        return None


@dataclass
class Recorder:
    requests: list[tuple[str, str]] = field(default_factory=list)

    def record(self, request: httpx.Request) -> None:
        self.requests.append((str(request.url), request.content.decode("utf-8")))


@dataclass
class Fixture:
    supplied: Recorder = field(default_factory=Recorder)
    factory: Recorder = field(default_factory=Recorder)
    factory_calls: list[tuple[str, int | None]] = field(default_factory=list)
    logging: FakeLogging = field(default_factory=FakeLogging)

    def used(self) -> Recorder:
        return self.factory if self.factory.requests else self.supplied


def transport(probe: Probe, recorder: Recorder) -> httpx.MockTransport:
    def handle(request: httpx.Request) -> httpx.Response:
        recorder.record(request)
        if probe.reply.failure == "timeout":
            raise httpx.ReadTimeout("read timed out")
        if probe.reply.failure == "failed":
            raise httpx.ReadError("the transport failed")
        return httpx.Response(
            probe.reply.status,
            headers=probe.reply.headers,
            content=probe.reply.body.encode("utf-8"),
        )

    return httpx.MockTransport(handle)


@contextmanager
def harness(probe: Probe, fixture: Fixture) -> Generator[None, None, None]:
    secrets: dict[str, str | None] = {
        "AWS_REGION_NAME": probe.environment.region_name,
        "AWS_REGION": probe.environment.region,
        "AWS_BEDROCK_RUNTIME_ENDPOINT": probe.environment.runtime_endpoint,
    }

    def secret_lookup(name: str, default: str | None = None) -> str | None:
        value = secrets.get(name)
        return default if value is None else value

    def bearer_lookup(name: str, token: str | None) -> str | None:
        return token if name == "AWS_BEARER_TOKEN_BEDROCK" else None

    def resolver(_self: object, _auth: object, _region: object) -> BotoCredentials:
        credentials = probe.credentials
        return BotoCredentials(
            access_key=credentials.access_key_id,
            secret_key=credentials.secret_access_key,
            token=credentials.session_token,
        )

    def sync_factory(*_args: object, **_kwargs: object) -> HTTPHandler:
        fixture.factory_calls.append(("sync", None))
        return HTTPHandler(client=httpx.Client(transport=transport(probe, fixture.factory)))

    def async_factory(*_args: object, **_kwargs: object) -> AsyncHTTPHandler:
        timeout = probe.timeout_ms
        fixture.factory_calls.append(("async", timeout))
        return AsyncHTTPHandler(transport=transport(probe, fixture.factory))

    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(base_aws_llm, "get_secret", secret_lookup),
        patch.object(
            base_aws_llm,
            "get_secret_str",
            partial(bearer_lookup, token=probe.configured_bearer_token),
        ),
        patch(
            "boto3.Session",
            lambda: SimpleNamespace(region_name=probe.environment.session_region),
        ),
        patch.object(
            botocore.auth,
            "get_current_datetime",
            lambda: datetime.fromtimestamp(probe.now, tz=UTC).replace(tzinfo=None),
        ),
        patch.object(BedrockImageGeneration, "resolve_credentials", resolver),
        patch.object(image_handler, "_get_httpx_client", sync_factory),
        patch.object(image_handler, "get_async_httpx_client", async_factory),
    ):
        yield


def prepare_value(probe: Probe) -> Json:
    fixture = Fixture()
    logging = cast("Logging", fixture.logging)
    with harness(probe, fixture):
        prepared = BedrockImageGeneration()._prepare_request(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            model=probe.model,
            optional_params=dict(probe.optional_params),
            api_base=probe.api_base,
            extra_headers=probe.extra_headers,
            logging_obj=logging,
            prompt=probe.prompt,
            api_key=probe.api_key,
        )
    pre_call = fixture.logging.calls[0][1]
    arguments = cast("dict[str, Json]", pre_call.get("additional_args", {}))
    return {
        "endpoint_url": prepared.endpoint_url,
        "prepared_url": prepared.prepped.url,  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        "headers": dict(prepared.prepped.headers),  # pyright: ignore[reportUnknownArgumentType]
        "body": prepared.body.decode("utf-8"),
        "data": cast("dict[str, Json]", prepared.data),
        "pre_call": {
            "input": pre_call.get("input"),
            "api_base": arguments.get("api_base"),
            "complete_input_dict": arguments.get("complete_input_dict"),
        },
    }


def generation(probe: Probe, fixture: Fixture) -> ImageResponse:
    client = (
        HTTPHandler(client=httpx.Client(transport=transport(probe, fixture.supplied)))
        if probe.supplied == "sync"
        else AsyncHTTPHandler(transport=transport(probe, fixture.supplied))
        if probe.supplied == "async"
        else None
    )
    logging = cast("Logging", fixture.logging)
    with harness(probe, fixture):
        pending = BedrockImageGeneration().image_generation(  # pyright: ignore[reportUnknownMemberType]
            model=probe.model,
            prompt=probe.prompt,
            model_response=ImageResponse(created=probe.created),
            optional_params=dict(probe.optional_params),
            logging_obj=logging,
            timeout=None if probe.timeout_ms is None else probe.timeout_ms / 1000,
            aimg_generation=probe.aimg_generation,
            api_base=probe.api_base,
            extra_headers=probe.extra_headers,
            client=client,
            api_key=probe.api_key,
        )
        if probe.aimg_generation:
            return asyncio.run(cast("Coroutine[object, object, ImageResponse]", pending))
        return cast("ImageResponse", pending)


def call_value(probe: Probe) -> Json:
    fixture = Fixture()
    result = generation(probe, fixture)
    data: list[Json] = [
        {"b64_json": image.b64_json} for image in result.data or []
    ]
    return {
        "created": result.created,
        "data": data,
        "posted_url": fixture.used().requests[0][0],
        "posted_body": json.loads(fixture.used().requests[0][1]),
        "post_call": fixture.logging.post_call_value(),
    }


def selection_value(probe: Probe) -> Json:
    fixture = Fixture()
    generation(probe, fixture)
    factory: str = fixture.factory_calls[-1][0] if fixture.factory_calls else "none"
    timeout = fixture.factory_calls[-1][1] if fixture.factory_calls else None
    return {
        "factory": factory,
        "timeout_ms": timeout,
        "supplied_requests": len(fixture.supplied.requests),
        "factory_requests": len(fixture.factory.requests),
        "posted_url": fixture.used().requests[0][0],
    }


def runtime_value(probe: Probe) -> Json:
    value = cast("dict[str, Json]", prepare_value(probe))
    fixture = Fixture()
    generation(probe, fixture)
    value["posted_url"] = fixture.used().requests[0][0]
    return value


def error_value(probe: Probe) -> Json:
    fixture = Fixture()
    try:
        generation(probe, fixture)
    except Exception as error:  # noqa: BLE001 - the helper reports the stable error fields
        carried: object = getattr(error, "status_code", None)
        status = carried if isinstance(carried, int) and not isinstance(carried, bool) else None
        return {
            "ok": False,
            "status": status,
            "post_call": fixture.logging.post_call_value() is not None,
        }
    return {"ok": True, "status": None, "post_call": True}


HANDLERS: dict[str, Callable[[Probe], Json]] = {
    "image_family_dispatch": call_value,
    "image_runtime_url": runtime_value,
    "signed_json_body": prepare_value,
    "extra_headers_outside_body": prepare_value,
    "base64_image_result": call_value,
    "image_http_error": error_value,
    "image_transport_selection": selection_value,
}


def run(input_value: dict[str, Json]) -> Json:
    name, argument = next(iter(input_value.items()))
    return [HANDLERS[name](probe) for probe in Probes.model_validate(argument).probes]
