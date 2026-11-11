import asyncio
import base64
import contextvars
import os
import threading
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
from pydantic import JsonValue

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import BedrockCreateBatchRequest
Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, Json]
ServiceName: TypeAlias = Literal[
    "bedrock", "sagemaker", "bedrock-agentcore", "s3vectors", "aws-external-anthropic"
]


def fixed_clock(epoch: int) -> Callable[[], datetime]:
    return lambda: datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)


def secret_lookup(_name: str, default: str | None = None) -> str | None:
    return default


def bearer_lookup(name: str, token: str | None) -> str | None:
    return token if name == "AWS_BEARER_TOKEN_BEDROCK" else None


def session_factory() -> SimpleNamespace:
    return SimpleNamespace(region_name=None)


class Resolver:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials
        self.calls = 0

    def __call__(self, _auth_params: object, _region: object) -> Credentials:
        self.calls += 1
        return self.credentials


class NonAwsConfig:
    pass


class AwsConfig(SignsRequestsWithAWS):
    pass


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def one(value: Json, name: str) -> tuple[str, Json]:
    if not isinstance(value, dict) or len(value) != 1:
        raise ValueError(f"{name} must have exactly one member")
    return next(iter(value.items()))


def now_of(case: Params) -> int:
    value = case.get("now")
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError("now must be an integer")
    return value


def headers_of(value: Json) -> dict[str, str | None]:
    raw = object_value(value, "headers")
    return {name: optional_string(entry, name) for name, entry in raw.items()}


def resolved_headers_of(value: Json) -> dict[str, str]:
    raw = object_value(value, "headers")
    return {name: string_value(entry, name) for name, entry in raw.items()}


def optional_params_of(value: Json) -> dict[str, Json]:
    return object_value(value, "optional_params")


def credentials_of(value: Json) -> Credentials:
    params = object_value(value, "credentials")
    return Credentials(
        access_key=string_value(params.get("access_key_id"), "access_key_id"),
        secret_key=string_value(params.get("secret_access_key"), "secret_access_key"),
        token=optional_string(params.get("session_token"), "session_token"),
    )


def optional_credentials(value: Json) -> Credentials | None:
    return None if value is None else credentials_of(value)


def body_of(value: Json) -> str | bytes:
    kind, payload = one(value, "data")
    if kind == "text":
        return string_value(payload, "data.text")
    if kind == "base64":
        return base64.b64decode(string_value(payload, "data.base64"), validate=True)
    raise ValueError(f"unknown data kind: {kind}")


def batch_data_of(value: Json) -> str | dict[str, Json]:
    if isinstance(value, str):
        return value
    return object_value(value, "data")


def headers_json(headers: Mapping[str, object]) -> dict[str, Json]:
    return {name: entry for name, entry in headers.items() if isinstance(entry, str)}


def body_json(payload: object) -> str:
    if isinstance(payload, str):
        return base64.b64encode(payload.encode()).decode()
    if isinstance(payload, bytes):
        return base64.b64encode(payload).decode()
    return ""


def signed_json(method: str, url: str, headers: Mapping[str, object], body: object) -> Json:
    return cast(
        Json,
        {
            "ok": True,
            "method": method,
            "url": url,
            "headers": headers_json(headers),
            "body": body_json(body),
        },
    )


@contextmanager
def clock(epoch: int) -> Generator[None, None, None]:
    with patch.object(botocore.auth, "get_current_datetime", fixed_clock(epoch)):
        yield


@contextmanager
def harness(
    configured_bearer: str | None, resolver: Resolver | None
) -> Generator[None, None, None]:
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(base_aws_llm, "get_secret", secret_lookup),
        patch.object(
            base_aws_llm, "get_secret_str", partial(bearer_lookup, token=configured_bearer)
        ),
        patch("boto3.Session", session_factory),
    ):
        if resolver is None:
            yield
        else:
            with patch.object(BaseAWSLLM, "resolve_credentials", resolver):
                yield


def request_headers(case: Params) -> Json:
    supports = case.get("supports_bearer_token", True)
    if not isinstance(supports, bool):
        raise ValueError("supports_bearer_token must be a boolean")
    configured = optional_string(case.get("configured_bearer_token"), "configured_bearer_token")
    with harness(configured, None), clock(now_of(case)):
        get_headers = BaseAWSLLM().get_request_headers  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        prepped = get_headers(
            credentials=optional_credentials(case.get("credentials")),
            aws_region_name=string_value(case.get("region"), "region"),
            extra_headers=resolved_headers_of(case.get("extra_headers")),
            endpoint_url=string_value(case.get("endpoint"), "endpoint"),
            data=body_of(case.get("data")),
            headers=headers_of(case.get("headers")),
            api_key=optional_string(case.get("api_key"), "api_key"),
            supports_bearer_token=supports,
        )
    return signed_json("POST", prepped.url, prepped.headers, prepped.body)  # pyright: ignore[reportUnknownArgumentType]


def service_request(case: Params) -> Json:
    resolver = Resolver(credentials_of(case.get("credentials")))
    configured = optional_string(case.get("configured_bearer_token"), "configured_bearer_token")
    with harness(configured, resolver), clock(now_of(case)):
        headers, body = BaseAWSLLM()._sign_request(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            service_name=cast(ServiceName, string_value(case.get("service_name"), "service_name")),
            headers=headers_of(case.get("headers")),
            optional_params=optional_params_of(case.get("optional_params")),
            request_data=object_value(case.get("request_data"), "request_data"),
            api_base=string_value(case.get("api_base"), "api_base"),
            model=optional_string(case.get("model"), "model"),
            api_key=optional_string(case.get("api_key"), "api_key"),
        )
    return signed_json("POST", string_value(case.get("api_base"), "api_base"), headers, body)  # pyright: ignore[reportUnknownArgumentType]


def json_post(case: Params) -> Json:
    credentials = credentials_of(case.get("credentials"))
    with clock(now_of(case)):
        prepped = sign_aws_json_post(
            get_credentials=lambda: credentials,
            service_name=string_value(case.get("service_name"), "service_name"),
            aws_region_name=optional_string(case.get("region"), "region"),
            url=string_value(case.get("url"), "url"),
            body=string_value(case.get("body"), "body"),
            headers=resolved_headers_of(case.get("headers")),
        )
    return signed_json("POST", prepped.url, prepped.headers, prepped.body)  # pyright: ignore[reportUnknownArgumentType]


def request_target(case: Params) -> Json:
    resolver = Resolver(credentials_of(case.get("credentials")))
    with harness(None, resolver):
        params = dict(optional_params_of(case.get("optional_params")))
        target = BaseAWSLLM()._get_boto_credentials_from_optional_params(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            params,
            optional_string(case.get("model"), "model"),
            bearer_token=optional_string(case.get("bearer_token"), "bearer_token"),
        )
    credentials: Json = (
        None
        if isinstance(target, BearerRequestTarget)
        else cast(
            Json,
            [
                target.credentials.access_key,
                target.credentials.secret_key,
                target.credentials.token,
            ],
        )
    )  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
    return cast(
        Json,
        {
            "ok": True,
            "region": target.aws_region_name,
            "endpoint": target.aws_bedrock_runtime_endpoint,
            "credentials": credentials,
            "remaining": cast(Json, params),
            "resolved": resolver.calls,
        },
    )


def batch_request(case: Params) -> Json:
    resolver = Resolver(credentials_of(case.get("credentials")))
    method = optional_string(case.get("method"), "method")
    data = cast(
        "str | dict[str, object] | BedrockCreateBatchRequest", batch_data_of(case.get("data"))
    )
    endpoint_url = string_value(case.get("endpoint_url"), "endpoint_url")
    with harness(None, resolver), clock(now_of(case)):
        sign = CommonBatchFilesUtils().sign_aws_request  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        signed: tuple[Mapping[str, object], object] = sign(  # pyright: ignore[reportUnknownVariableType]
            service_name=string_value(case.get("service_name"), "service_name"),
            data=data,
            endpoint_url=endpoint_url,
            optional_params=optional_params_of(case.get("optional_params")),
            method=method if method is not None else "POST",
        )
    return signed_json(
        method.upper() if method is not None else "POST", endpoint_url, signed[0], signed[1]
    )


def value_json(kind: str, observed: Mapping[str, object]) -> Json:
    return cast(
        Json,
        {
            "kind": kind,
            "failed": False,
            "value": observed["value"],
            "context": observed["context"],
            "context_preserved": observed["context_preserved"],
            "off_caller_thread": observed["off_caller_thread"],
        },
    )


async def off_loop_steps(context: str, value: str, failure: str) -> list[Json]:
    marker: contextvars.ContextVar[str] = contextvars.ContextVar(
        "bedrock_signing_context", default=""
    )
    marker.set(context)
    loop_thread = threading.get_ident()

    def observe() -> dict[str, object]:
        return {
            "value": value,
            "context": marker.get(),
            "context_preserved": marker.get() == context,
            "off_caller_thread": threading.get_ident() != loop_thread,
        }

    def failing() -> dict[str, object]:
        raise RuntimeError(failure)

    run = await run_aws_signing(observe)
    inline = await sign_request_off_loop_if_aws(NonAwsConfig(), observe)
    offloaded = await sign_request_off_loop_if_aws(AwsConfig(), observe)
    failure_step: Json
    try:
        await run_aws_signing(failing)
    except RuntimeError as error:
        failure_step = cast(Json, {"kind": "failure", "failed": True, "value": str(error)})
    else:
        failure_step = cast(Json, {"kind": "failure", "failed": False})
    return [
        value_json("run", run),
        value_json("inline", inline),
        value_json("offloaded", offloaded),
        failure_step,
    ]


def off_loop(case: Params) -> Json:
    steps = asyncio.run(
        off_loop_steps(
            string_value(case.get("context"), "context"),
            string_value(case.get("value"), "value"),
            string_value(case.get("failure"), "failure"),
        )
    )
    return cast(Json, {"ok": True, "steps": steps})


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "request_headers": request_headers,
    "service_request": service_request,
    "json_post": json_post,
    "request_target": request_target,
    "batch_request": batch_request,
    "off_loop": off_loop,
}


def run_call(call: Json) -> Json:
    kind, payload = one(call, "call")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown call: {kind}")
    try:
        return handler(object_value(payload, kind))
    except Exception:  # noqa: BLE001 - a step reports every fixture failure
        return cast(Json, {"ok": False, "status": None})


def run(input_value: Params) -> Json:
    calls = list_value(input_value.get("calls"), "calls")
    _ = litellm
    return cast(Json, {"steps": [run_call(call) for call in calls]})
