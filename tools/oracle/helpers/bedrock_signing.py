import asyncio
import contextvars
import os
import threading
from base64 import b64decode, b64encode
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from functools import partial
from types import SimpleNamespace
from typing import TYPE_CHECKING, Literal, TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import litellm
from botocore.credentials import Credentials
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import (
    BaseAWSLLM,
    BearerRequestTarget,
    SignsRequestsWithAWS,
    run_aws_signing,
    sign_aws_json_post,
    sign_request_off_loop_if_aws,
)
from litellm.llms.bedrock.common_utils import CommonBatchFilesUtils
from pydantic import BaseModel, JsonValue, TypeAdapter

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import BedrockCreateBatchRequest
Json: TypeAlias = JsonValue
ServiceName: TypeAlias = Literal[
    "bedrock", "sagemaker", "bedrock-agentcore", "s3vectors", "aws-external-anthropic"
]


class Keys(BaseModel):
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None

    def to_boto(self) -> Credentials:
        return Credentials(
            access_key=self.access_key_id,
            secret_key=self.secret_access_key,
            token=self.session_token,
        )


class RequestHeaders(BaseModel):
    credentials: Keys | None = None
    region: str
    extra_headers: dict[str, str]
    endpoint: str
    data: dict[Literal["text", "base64"], str]
    headers: dict[str, str | None]
    api_key: str | None = None
    configured_bearer_token: str | None = None
    supports_bearer_token: bool = True
    now: int


class ServiceRequest(BaseModel):
    credentials: Keys
    service_name: ServiceName
    headers: dict[str, str | None]
    optional_params: dict[str, Json]
    request_data: dict[str, Json]
    api_base: str
    model: str | None = None
    api_key: str | None = None
    configured_bearer_token: str | None = None
    now: int


class JsonPost(BaseModel):
    credentials: Keys
    service_name: str
    region: str | None = None
    url: str
    body: str
    headers: dict[str, str]
    now: int


class RequestTarget(BaseModel):
    credentials: Keys
    optional_params: dict[str, Json]
    model: str | None = None
    bearer_token: str | None = None


class BatchRequest(BaseModel):
    credentials: Keys
    service_name: str
    data: str | dict[str, Json]
    endpoint_url: str
    optional_params: dict[str, Json]
    method: str | None = None
    now: int


class OffLoop(BaseModel):
    context: str
    value: str
    failure: str


def secret_lookup(_name: str, default: str | None = None) -> str | None:
    return default


def bearer_lookup(name: str, token: str | None) -> str | None:
    return token if name == "AWS_BEARER_TOKEN_BEDROCK" else None


class Resolver:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials
        self.calls = 0

    def __call__(self, _auth_params: object, _region: object) -> Credentials:
        self.calls += 1
        return self.credentials


def signed_json(method: str, url: str, headers: Mapping[str, object], body: object) -> Json:
    if isinstance(body, str):
        body = body.encode()
    return {
        "ok": True,
        "method": method,
        "url": url,
        "headers": {name: entry for name, entry in headers.items() if isinstance(entry, str)},
        "body": b64encode(body).decode() if isinstance(body, bytes) else "",
    }


@contextmanager
def clock(epoch: int) -> Generator[None, None, None]:
    frozen = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)
    with patch.object(botocore.auth, "get_current_datetime", lambda: frozen):
        yield


@contextmanager
def harness(bearer: str | None, resolver: Resolver | None) -> Generator[None, None, None]:
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(base_aws_llm, "get_secret", secret_lookup),
        patch.object(base_aws_llm, "get_secret_str", partial(bearer_lookup, token=bearer)),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
    ):
        if resolver is None:
            yield
        else:
            with patch.object(BaseAWSLLM, "resolve_credentials", resolver):
                yield


def request_headers(payload: Json) -> Json:
    case = RequestHeaders.model_validate(payload)
    encoded = case.data.get("base64")
    data = case.data["text"] if encoded is None else b64decode(encoded, validate=True)
    with harness(case.configured_bearer_token, None), clock(case.now):
        get_headers = BaseAWSLLM().get_request_headers  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        prepped = get_headers(
            credentials=case.credentials.to_boto() if case.credentials else None,
            aws_region_name=case.region,
            extra_headers=case.extra_headers,
            endpoint_url=case.endpoint,
            data=data,
            headers=case.headers,
            api_key=case.api_key,
            supports_bearer_token=case.supports_bearer_token,
        )
    return signed_json("POST", prepped.url, prepped.headers, prepped.body)  # pyright: ignore[reportUnknownArgumentType]


def service_request(payload: Json) -> Json:
    case = ServiceRequest.model_validate(payload)
    resolver = Resolver(case.credentials.to_boto())
    with harness(case.configured_bearer_token, resolver), clock(case.now):
        headers, body = BaseAWSLLM()._sign_request(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            service_name=case.service_name,
            headers=case.headers,
            optional_params=case.optional_params,
            request_data=case.request_data,
            api_base=case.api_base,
            model=case.model,
            api_key=case.api_key,
        )
    return signed_json("POST", case.api_base, headers, body)  # pyright: ignore[reportUnknownArgumentType]


def json_post(payload: Json) -> Json:
    case = JsonPost.model_validate(payload)
    credentials = case.credentials.to_boto()
    with clock(case.now):
        prepped = sign_aws_json_post(
            get_credentials=lambda: credentials,
            service_name=case.service_name,
            aws_region_name=case.region,
            url=case.url,
            body=case.body,
            headers=case.headers,
        )
    return signed_json("POST", prepped.url, prepped.headers, prepped.body)  # pyright: ignore[reportUnknownArgumentType]


def request_target(payload: Json) -> Json:
    case = RequestTarget.model_validate(payload)
    resolver = Resolver(case.credentials.to_boto())
    params = dict(case.optional_params)
    with harness(None, resolver):
        target = BaseAWSLLM()._get_boto_credentials_from_optional_params(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            params, case.model, bearer_token=case.bearer_token
        )
    held = None if isinstance(target, BearerRequestTarget) else target.credentials  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return {
        "ok": True,
        "region": target.aws_region_name,
        "endpoint": target.aws_bedrock_runtime_endpoint,
        "credentials": None if held is None else [held.access_key, held.secret_key, held.token],  # pyright: ignore[reportUnknownMemberType]
        "remaining": params,
        "resolved": resolver.calls,
    }


def batch_request(payload: Json) -> Json:
    case = BatchRequest.model_validate(payload)
    method = "POST" if case.method is None else case.method
    data = cast("str | dict[str, object] | BedrockCreateBatchRequest", case.data)
    with harness(None, Resolver(case.credentials.to_boto())), clock(case.now):
        sign = CommonBatchFilesUtils().sign_aws_request  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        signed: tuple[Mapping[str, object], object] = sign(  # pyright: ignore[reportUnknownVariableType]
            service_name=case.service_name,
            data=data,
            endpoint_url=case.endpoint_url,
            optional_params=case.optional_params,
            method=method,
        )
    return signed_json(method.upper(), case.endpoint_url, signed[0], signed[1])


async def off_loop_steps(case: OffLoop) -> list[Json]:
    marker: contextvars.ContextVar[str] = contextvars.ContextVar(
        "bedrock_signing_context", default=""
    )
    marker.set(case.context)
    loop_thread = threading.get_ident()

    def observe() -> dict[str, Json]:
        return {
            "value": case.value,
            "context": marker.get(),
            "context_preserved": marker.get() == case.context,
            "off_caller_thread": threading.get_ident() != loop_thread,
        }

    def failing() -> dict[str, Json]:
        raise RuntimeError(case.failure)

    steps: list[Json] = [
        {"kind": "run", "failed": False, **await run_aws_signing(observe)},
        {
            "kind": "inline",
            "failed": False,
            **await sign_request_off_loop_if_aws(object(), observe),
        },
        {
            "kind": "offloaded",
            "failed": False,
            **await sign_request_off_loop_if_aws(SignsRequestsWithAWS(), observe),
        },
    ]
    try:
        await run_aws_signing(failing)
    except RuntimeError as error:
        steps.append({"kind": "failure", "failed": True, "value": str(error)})
    else:
        steps.append({"kind": "failure", "failed": False})
    return steps


def off_loop(payload: Json) -> Json:
    return {"ok": True, "steps": asyncio.run(off_loop_steps(OffLoop.model_validate(payload)))}


HANDLERS: dict[str, Callable[[Json], Json]] = {
    "request_headers": request_headers,
    "service_request": service_request,
    "json_post": json_post,
    "request_target": request_target,
    "batch_request": batch_request,
    "off_loop": off_loop,
}


def run_call(call: Json) -> Json:
    if not isinstance(call, dict) or len(call) != 1:
        raise ValueError("call must have exactly one member")
    kind, payload = next(iter(call.items()))
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown call: {kind}")
    try:
        return handler(payload)
    except Exception:  # noqa: BLE001 - a step reports every fixture failure
        return {"ok": False, "status": None}


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm
    return {
        "steps": [
            run_call(call) for call in TypeAdapter(list[Json]).validate_python(input_value["calls"])
        ]
    }
