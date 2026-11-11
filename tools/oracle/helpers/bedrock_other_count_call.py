import asyncio
import json
import os
from collections.abc import Generator, Mapping, Sequence
from contextlib import ExitStack, contextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Final, Literal, TypeAlias, cast
from unittest import mock

import botocore.auth
import httpx
import litellm
from botocore.credentials import Credentials
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.common_utils import BedrockModelInfo
from litellm.llms.bedrock.count_tokens.bedrock_token_counter import BedrockTokenCounter
from litellm.llms.bedrock.count_tokens.handler import BedrockCountTokensHandler
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import TokenCountResponse
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = JsonValue

HANDLER_CLIENT: Final = "litellm.llms.bedrock.count_tokens.handler.get_async_httpx_client"
HANDLER_LOGGER: Final = "litellm.llms.bedrock.count_tokens.handler.verbose_logger"
COUNTER_LOGGER: Final = "litellm.llms.bedrock.count_tokens.bedrock_token_counter.verbose_logger"
PROCESSING_PREFIX: Final = "CountTokens processing error: "
INTERNAL_SERVER_ERROR: Final = 500
REPORTED_HEADERS: Final = (
    "content-type",
    "authorization",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
)
DEBUG_SUBJECTS: Final[dict[str, str]] = {
    "Processing CountTokens request for resolved model: %s": "processing",
    "Retrieved AWS region: %s": "region",
    "Transformed request: %s": "transformed",
    "Making request to: %s": "endpoint",
    "Response status: %s": "status",
    "Bedrock response: %s": "reply",
    "Final response: %s": "final",
}
ERROR_SUBJECTS: Final[dict[str, str]] = {
    "AWS Bedrock error: %s": "AWS Bedrock error",
    "HTTP error in CountTokens handler: %s": "HTTP error in CountTokens handler",
    "Error in CountTokens handler: %s": "Error in CountTokens handler",
}
API_WARNING: Final = "Bedrock CountTokens API error: status=%s, message=%s"
GENERIC_WARNING: Final = "Error calling Bedrock CountTokens API: %s"


class EnvironmentModel(BaseModel):
    model_config = ConfigDict(extra="forbid")
    region_name: str | None = None
    region: str | None = None
    session_region: str | None = None
    runtime_endpoint: str | None = None
    bearer_token: str | None = None


class ReplyModel(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status: int = 200
    body: str = "{}"
    headers: dict[str, str] = {}
    failure: bool = False


class CallModel(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str
    messages: Json = None
    contents: Json = None
    tools: Json = None
    system: Json = None
    request_model: str = ""
    deployment: dict[str, Json] | None = None
    environment: EnvironmentModel = EnvironmentModel()
    provider: str | None = None
    handler: Literal["real", "none", "empty_object"] = "real"
    reply: ReplyModel = ReplyModel()
    now: int


class CallsModel(BaseModel):
    model_config = ConfigDict(extra="forbid")
    calls: list[CallModel] = []


class Provider:
    def __init__(self, reply: ReplyModel) -> None:
        self.reply = reply
        self.first: Json = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        if self.first is None:
            timeout = cast(dict[str, float], request.extensions["timeout"])
            sent = {name.lower(): value for name, value in request.headers.items()}
            self.first = {
                "method": request.method,
                "url": str(request.url),
                "body": request.content.decode(),
                "timeout": float(timeout["read"]),
                "headers": {name: sent[name] for name in REPORTED_HEADERS if name in sent},
            }
        if self.reply.failure:
            raise httpx.ReadTimeout("fixture transport failure", request=request)
        return httpx.Response(
            self.reply.status,
            content=self.reply.body.encode(),
            headers=self.reply.headers,
            request=request,
        )

    def client(self) -> AsyncHTTPHandler:
        handler = AsyncHTTPHandler()
        handler.client = httpx.AsyncClient(transport=httpx.MockTransport(self.handle))
        return handler


class Events:
    def __init__(self) -> None:
        self.debugs: list[Json] = []
        self.errors: list[Json] = []
        self.warnings: list[Json] = []

    def debug(self, message: str, *args: object) -> None:
        value: Json = to_json(args[0]) if args else None
        self.debugs.append(cast(Json, {"subject": DEBUG_SUBJECTS[message], "value": value}))

    def error(self, message: str, *args: object) -> None:
        _ = args
        self.errors.append(cast(Json, {"subject": ERROR_SUBJECTS[message]}))

    def warning(self, message: str, *args: object) -> None:
        if message == API_WARNING:
            self.warnings.append(
                cast(Json, {"subject": "Bedrock CountTokens API error", "status": args[0]})
            )
        else:
            self.warnings.append(
                cast(Json, {"subject": "Error calling Bedrock CountTokens API", "status": None})
            )


class Resolver:
    def __init__(self, params: dict[str, Json]) -> None:
        self.params = params
        self.calls = 0

    def __call__(self, _auth_params: object, _region: object) -> Credentials:
        self.calls += 1
        access = self.params.get("aws_access_key_id")
        secret = self.params.get("aws_secret_access_key")
        token = self.params.get("aws_session_token")
        if isinstance(access, str) and isinstance(secret, str):
            return Credentials(
                access_key=access,
                secret_key=secret,
                token=token if isinstance(token, str) else None,
            )
        return Credentials(
            access_key="AKIDEXAMPLE",
            secret_key="fixture-secret-key",  # noqa: S106 - an oracle fixture, not a real key
            token=None,
        )


def to_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def error_message_kind(message: str | None, status: int | None) -> str | None:
    if message is None:
        return None
    if status == INTERNAL_SERVER_ERROR:
        return "processing" if message.startswith(PROCESSING_PREFIX) else "generic"
    return message


def result_json(result: TokenCountResponse) -> Json:
    return cast(
        Json,
        {
            "total_tokens": result.total_tokens,
            "request_model": result.request_model,
            "model_used": result.model_used,
            "tokenizer_type": result.tokenizer_type,
            "original_response": cast(Json, result.original_response),
            "error": result.error,
            "error_message": error_message_kind(result.error_message, result.status_code),
            "status_code": result.status_code,
        },
    )


@contextmanager
def clock(epoch: int) -> Generator[None, None, None]:
    frozen = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)
    with mock.patch.object(botocore.auth, "get_current_datetime", return_value=frozen):
        yield


@contextmanager
def harness(
    call: CallModel, provider: Provider, resolver: Resolver, events: Events
) -> Generator[None, None, None]:
    environment = call.environment
    values: Final[dict[str, str | None]] = {
        "AWS_REGION_NAME": environment.region_name,
        "AWS_REGION": environment.region,
        "AWS_BEDROCK_RUNTIME_ENDPOINT": environment.runtime_endpoint,
        "AWS_BEARER_TOKEN_BEDROCK": environment.bearer_token,
    }

    def secret(name: str, default: str | None = None) -> str | None:
        value = values.get(name)
        return value if value is not None else default

    with ExitStack() as stack:
        stack.enter_context(mock.patch.dict(os.environ, {}, clear=True))
        stack.enter_context(mock.patch.object(base_aws_llm, "get_secret", secret))
        stack.enter_context(mock.patch.object(base_aws_llm, "get_secret_str", values.get))
        stack.enter_context(
            mock.patch(
                "boto3.Session",
                return_value=SimpleNamespace(region_name=environment.session_region),
            )
        )
        stack.enter_context(mock.patch.object(BaseAWSLLM, "resolve_credentials", resolver))
        stack.enter_context(mock.patch(HANDLER_CLIENT, return_value=provider.client()))
        stack.enter_context(mock.patch(HANDLER_LOGGER, new=events))
        stack.enter_context(mock.patch(COUNTER_LOGGER, new=events))
        stack.enter_context(clock(call.now))
        yield


async def count_once(counter: BedrockTokenCounter, call: CallModel) -> TokenCountResponse | None:
    messages = cast(Sequence[Mapping[str, object]] | None, call.messages)
    contents = cast(Sequence[Mapping[str, object]] | None, call.contents)
    tools = cast(Sequence[Mapping[str, object]] | None, call.tools)
    return await counter.count_tokens(
        model_to_use=call.model,
        messages=messages,
        contents=contents,
        deployment=call.deployment,
        request_model=call.request_model,
        tools=tools,
        system=call.system,
    )


def counter_for(*, factory: bool) -> BedrockTokenCounter:
    counter = BedrockModelInfo().get_token_counter() if factory else BedrockTokenCounter()
    if not isinstance(counter, BedrockTokenCounter):
        raise TypeError("the Bedrock model info did not return the native token counter")
    return counter


def one_call(call: CallModel, *, factory: bool) -> Json:
    provider = Provider(call.reply)
    params = {} if call.deployment is None else call.deployment.get("litellm_params", {})
    resolver = Resolver(cast(dict[str, Json], params))
    events = Events()
    counter = counter_for(factory=factory)
    selected = counter.should_use_token_counting_api(call.provider)
    replacement = {
        "none": mock.AsyncMock(return_value=None),
        "empty_object": mock.AsyncMock(return_value={}),
    }.get(call.handler)
    with harness(call, provider, resolver, events):
        if replacement is None:
            result = asyncio.run(count_once(counter, call))
        else:
            patched = mock.patch.object(
                BedrockCountTokensHandler, "handle_count_tokens_request", replacement
            )
            with patched:
                result = asyncio.run(count_once(counter, call))
    return cast(
        Json,
        {
            "selected": selected,
            "result": None if result is None else result_json(result),
            "request": provider.first,
            "debug": events.debugs,
            "errors": events.errors,
            "warnings": events.warnings,
            "resolved": resolver.calls,
        },
    )


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm
    operation, argument = next(iter(input_value.items()))
    calls = CallsModel.model_validate(argument)
    factory = operation == "counter_factory"
    return cast(Json, {"calls": [one_call(call, factory=factory) for call in calls.calls]})
