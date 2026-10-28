import json
import os
from collections.abc import Callable, Iterator, Mapping, Sequence
from typing import TYPE_CHECKING, Protocol, TypeAlias, cast
from unittest import mock

import httpx
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.common_utils import BedrockError, error_response_text
from pydantic import BaseModel, ConfigDict, TypeAdapter

if TYPE_CHECKING:
    from litellm.types.llms.bedrock import AwsSessionTag

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Resolved: TypeAlias = Mapping[str, "str | bool | tuple[AwsSessionTag, ...] | None"]

ENVIRONMENT = (
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION_NAME",
    "AWS_SESSION_NAME",
    "AWS_PROFILE_NAME",
    "AWS_ROLE_NAME",
    "AWS_WEB_IDENTITY_TOKEN",
    "AWS_STS_ENDPOINT",
    "AWS_EXTERNAL_ID",
    "AWS_ROLE_ARN",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_BEARER_TOKEN_BEDROCK",
)
RESOLVED = (
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_region_name",
    "aws_session_name",
    "aws_profile_name",
    "aws_role_name",
    "aws_web_identity_token",
    "aws_sts_endpoint",
    "aws_external_id",
    "aws_session_tags",
)
PROBE_ROLE = "arn:aws:iam::000000000000:role/oracle"
BYTES: TypeAdapter[list[int]] = TypeAdapter(list[int])


class Deployment(BaseModel):
    model_config = ConfigDict(extra="forbid")
    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None
    aws_region_name: str | None = None
    aws_session_name: str | None = None
    aws_profile_name: str | None = None
    aws_role_name: str | None = None
    aws_web_identity_token: str | None = None
    aws_sts_endpoint: str | None = None
    aws_external_id: str | None = None
    aws_session_tags: object = None


class SettingsCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    deployments: list[Deployment]
    environment: dict[str, str]


class RequestLine(BaseModel):
    model_config = ConfigDict(extra="forbid")
    method: str
    url: str


class Unread(BaseModel):
    model_config = ConfigDict(extra="forbid")
    reason_phrase: str


class ReplyBody(BaseModel):
    model_config = ConfigDict(extra="forbid")
    read: str | None = None
    unread: Unread | None = None


class Reply(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status_code: int
    request: RequestLine
    headers: list[tuple[str, str]]
    body: ReplyBody


class HeaderSet(BaseModel):
    model_config = ConfigDict(extra="forbid")
    http: list[tuple[str, str]] | None = None
    mapping: list[tuple[str, object]] | None = None


class ErrorCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status_code: int
    message: str
    headers: HeaderSet | None
    request: RequestLine | None
    response: Reply | None
    body: dict[str, object] | None
    status_code_is_synthesized: bool


class ErrorView(Protocol):
    status_code: int
    status_code_is_synthesized: bool
    message: str
    headers: dict[str, object] | httpx.Headers | None
    request: httpx.Request
    response: httpx.Response
    body: dict[str, object] | None


class ResolvedInputs:
    def __init__(self) -> None:
        self.values: dict[str, Json] = {}

    def __call__(self, credential_args: Resolved, _fetcher: object) -> None:
        self.values = {name: to_json(credential_args[name]) for name in RESOLVED}


class EnvironmentReads:
    def __init__(self) -> None:
        self.values: dict[str, Json] = {}

    def __call__(self, key: str, default: str | None = None) -> str | None:
        value = os.environ.get(key, default)
        self.values[key] = value
        return value


class SecretReads:
    def __init__(self, read: Callable[[str], str | None]) -> None:
        self.read: Callable[[str], str | None] = read
        self.values: dict[str, Json] = {}

    def __call__(self, secret_name: str) -> str | None:
        value = self.read(secret_name)
        self.values[secret_name] = value
        return value


class UnreadStream(httpx.SyncByteStream):
    def __iter__(self) -> Iterator[bytes]:
        yield b"unread"


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def check_environment(environment: dict[str, str]) -> None:
    if {name: os.environ[name] for name in ENVIRONMENT if name in os.environ} != environment:
        raise ValueError("the call environment must match the case env")


def keep_tags(raw_tags: object) -> object:
    return raw_tags


def unreachable_sts(*_: object) -> None:
    raise RuntimeError("the oracle has no STS")


def every_flow(*_: object) -> bool:
    return True


def snapshot(deployment: Deployment) -> Json:
    credentials = base_aws_llm.BaseAWSLLM()
    resolved = ResolvedInputs()
    tags = cast("Sequence[AwsSessionTag] | None", deployment.aws_session_tags)
    # B590 picks the credential flow and validates the tags; one cached flow reaches the inputs
    with (
        mock.patch.object(base_aws_llm, "_canonical_aws_session_tags", keep_tags),
        mock.patch.object(credentials, "_is_auth_with_web_identity_token", every_flow),
        mock.patch.object(credentials, "_get_or_set_cached_credentials", resolved),
    ):
        _ = credentials.get_credentials(
            aws_access_key_id=deployment.aws_access_key_id,
            aws_secret_access_key=deployment.aws_secret_access_key,
            aws_session_token=deployment.aws_session_token,
            aws_region_name=deployment.aws_region_name,
            aws_session_name=deployment.aws_session_name,
            aws_profile_name=deployment.aws_profile_name,
            aws_role_name=deployment.aws_role_name,
            aws_web_identity_token=deployment.aws_web_identity_token,
            aws_sts_endpoint=deployment.aws_sts_endpoint,
            aws_external_id=deployment.aws_external_id,
            aws_session_tags=tags,
        )
    environment = EnvironmentReads()
    with mock.patch.object(os, "getenv", environment), mock.patch("boto3.client", unreachable_sts):
        _ = credentials._is_already_running_as_role(PROBE_ROLE)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    secrets = SecretReads(base_aws_llm.get_secret_str)
    with mock.patch.object(base_aws_llm, "get_secret_str", secrets):
        _ = base_aws_llm.bedrock_bearer_token(None)
    return {
        **resolved.values,
        "aws_role_arn": environment.values["AWS_ROLE_ARN"],
        "aws_web_identity_token_file": environment.values["AWS_WEB_IDENTITY_TOKEN_FILE"],
        "aws_bearer_token_bedrock": secrets.values["AWS_BEARER_TOKEN_BEDROCK"],
    }


def settings(argument: Json) -> Json:
    call = SettingsCall.model_validate(argument)
    check_environment(call.environment)
    return [snapshot(deployment) for deployment in call.deployments]


def header_value(value: object) -> object:
    match value:
        case "null":
            return None
        case {"text": str() as text}:
            return text
        case {"bytes": object() as encoded}:
            return bytes(BYTES.validate_python(encoded))
        case {"integer": int() as number}:
            return number
        case {"boolean": bool() as flag}:
            return flag
        case _:
            raise ValueError(f"unknown header value {value!r}")


def tagged_value(value: object) -> Json:
    match value:
        case None:
            return "null"
        case bool():
            return {"boolean": value}
        case int():
            return {"integer": value}
        case str():
            return {"text": value}
        case bytes():
            return {"bytes": list(value)}
        case _:
            raise TypeError(f"unexpected header value {value!r}")


def header_input(headers: HeaderSet | None) -> dict[str, object] | httpx.Headers | None:
    if headers is None:
        return None
    if headers.http is not None:
        return httpx.Headers(headers.http)
    return {name: header_value(value) for name, value in headers.mapping or []}


def header_output(headers: dict[str, object] | httpx.Headers | None) -> Json:
    if headers is None:
        return None
    if isinstance(headers, httpx.Headers):
        return {"http": raw_pairs(headers)}
    return {"mapping": [[name, tagged_value(value)] for name, value in headers.items()]}


def raw_pairs(headers: httpx.Headers) -> Json:
    encoding = headers.encoding
    return [[name.decode(encoding), value.decode(encoding)] for name, value in headers.raw]


def request_line(request: httpx.Request) -> Json:
    return {"method": request.method, "url": str(request.url)}


def http_request(line: RequestLine) -> httpx.Request:
    return httpx.Request(line.method, line.url)


def http_response(reply: Reply) -> httpx.Response:
    request = http_request(reply.request)
    if reply.body.unread is not None:
        phrase = reply.body.unread.reason_phrase.encode("ascii")
        return httpx.Response(
            reply.status_code,
            headers=reply.headers,
            stream=UnreadStream(),
            request=request,
            extensions={"reason_phrase": phrase},
        )
    content = (reply.body.read or "").encode()
    stream = httpx.ByteStream(content)
    response = httpx.Response(
        reply.status_code, headers=reply.headers, stream=stream, request=request
    )
    response.read()
    return response


def error(argument: Json) -> Json:
    call = ErrorCall.model_validate(argument)
    built = cast(
        "ErrorView",
        BedrockError(
            status_code=call.status_code,
            message=call.message,
            headers=header_input(call.headers),
            request=None if call.request is None else http_request(call.request),
            response=None if call.response is None else http_response(call.response),
            body=call.body,
            status_code_is_synthesized=call.status_code_is_synthesized,
        ),
    )
    response = built.response
    return {
        "status_code": built.status_code,
        "status_code_is_synthesized": built.status_code_is_synthesized,
        "message": built.message,
        "headers": header_output(built.headers),
        "request": request_line(built.request),
        "response": {
            "status_code": response.status_code,
            "request": request_line(response.request),
            "headers": raw_pairs(response.headers),
            "text": error_response_text(response),
        },
        "body": to_json(built.body),
    }


HANDLERS: dict[str, Callable[[Json], Json]] = {"settings": settings, "error": error}


def run(input_value: dict[str, Json]) -> Json:
    name, argument = next(iter(input_value.items()))
    return HANDLERS[name](argument)
