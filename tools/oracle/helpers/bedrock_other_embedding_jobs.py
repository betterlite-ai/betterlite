import asyncio
import json
import os
from collections.abc import Coroutine
from datetime import UTC, datetime
from functools import partial
from typing import NotRequired, Protocol, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L55
import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.embed import embedding as embedding_module
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.llms.bedrock import AwsAuthParams
from litellm.types.utils import LlmProviders
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

HEADER_NAMES = ("content-type", "x-amz-date", "x-amz-security-token", "authorization")

CLOCK_SECONDS = 1700000000


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str]


class Input(TypedDict):
    function: str
    invocation_arn: str
    region: str
    endpoint: NotRequired[str]
    environment: NotRequired[dict[str, str]]
    credentials: Keys
    status: NotRequired[int]
    body: NotRequired[str]
    logging: NotRequired[bool]


class PreArgs(TypedDict):
    api_base: str
    headers: dict[str, str]
    complete_input_dict: dict[str, str]
    request_str: str


class PostArgs(TypedDict):
    complete_input_dict: dict[str, str]


PRE_ARGS: TypeAdapter[PreArgs] = TypeAdapter(PreArgs)
POST_ARGS: TypeAdapter[PostArgs] = TypeAdapter(PostArgs)


class ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    text: str


class Recorder(Logging):
    def __init__(self) -> None:
        self.pre: list[dict[str, JsonValue]] = []
        self.post: list[dict[str, JsonValue]] = []

    @override
    def pre_call(self, *_args: JsonValue, **values: JsonValue) -> None:
        additional = PRE_ARGS.validate_python(values["additional_args"])
        header_names: list[str] = list(additional["headers"])
        self.pre.append(
            {
                "input": values["input"],
                "api_base": additional["api_base"],
                "header_names": JSON.validate_python(header_names),
                "request_str": additional["request_str"],
                "complete_input_dict": JSON.validate_python(additional["complete_input_dict"]),
            }
        )

    @override
    def post_call(self, *_args: JsonValue, **values: JsonValue) -> None:
        response = ResponseView.model_validate(values["original_response"])
        additional = POST_ARGS.validate_python(values["additional_args"])
        self.post.append(
            {
                "input": values["input"],
                "original_response": response.text,
                "complete_input_dict": JSON.validate_python(additional["complete_input_dict"]),
            }
        )


class Provider:
    def __init__(self, keys: Keys) -> None:
        self.keys = keys
        self.regions: list[str] = []

    def __call__(self, _auth: AwsAuthParams, region: str) -> "FakeCredentials":
        self.regions.append(region)
        return FakeCredentials(self.keys)


class FakeCredentials:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token: str | None = keys.get("session_token")


class Fixture:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.sent: list[dict[str, JsonValue]] = []
        self.factory: list[dict[str, JsonValue]] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        headers: list[JsonValue] = [
            [name.lower(), value]
            for name, value in dict(request.headers).items()
            if name.lower() in HEADER_NAMES
        ]
        self.sent.append(
            {"url": str(request.url), "headers": headers, "body": request.content.decode()}
        )
        return httpx.Response(
            self.status,
            headers={"content-type": "application/json"},
            content=self.body.encode(),
        )

    def client(self, llm_provider: LlmProviders) -> AsyncHTTPHandler:
        self.factory.append({"provider": llm_provider.value, "timeout": None})
        return AsyncHTTPHandler(transport=httpx.MockTransport(self.respond))


class StatusCall(Protocol):
    def __call__(
        self,
        invocation_arn: str,
        aws_region_name: str,
        *,
        logging_obj: Recorder | None,
        **kwargs: JsonValue,
    ) -> Coroutine[None, None, JsonValue]: ...


class Caller(embedding_module.BedrockEmbedding):
    async def status(self, value: Input, logging_obj: Recorder | None) -> JsonValue:
        kwargs: dict[str, JsonValue] = {
            "aws_access_key_id": value["credentials"]["access_key_id"],
            "aws_secret_access_key": value["credentials"]["secret_access_key"],
        }
        token = value["credentials"].get("session_token")
        if token is not None:
            kwargs["aws_session_token"] = token
        endpoint = value.get("endpoint")
        if endpoint is not None:
            kwargs["aws_bedrock_runtime_endpoint"] = endpoint
        call = cast(StatusCall, self._get_async_invoke_status)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L587-L667
        return await call(
            value["invocation_arn"], value["region"], logging_obj=logging_obj, **kwargs
        )


def set_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(environment)


def record_value(
    fixture: Fixture, provider: Provider, recorder: Recorder | None
) -> dict[str, JsonValue]:
    pre = None if recorder is None or not recorder.pre else recorder.pre[0]
    post = None if recorder is None or not recorder.post else recorder.post[0]
    return {
        "requests": JSON.validate_python(fixture.sent),
        "factory": JSON.validate_python(fixture.factory),
        "resolved_regions": JSON.validate_python(provider.regions),
        "pre": pre,
        "post": post,
    }


def run(value: Input) -> JsonValue:
    environment = value.get("environment")
    set_environment({} if environment is None else environment)
    provider = Provider(value["credentials"])
    BaseAWSLLM.resolve_credentials = provider  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    botocore.auth.get_current_datetime = partial(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1758
        datetime.fromtimestamp, CLOCK_SECONDS, tz=UTC
    )
    status = value.get("status")
    if status is None:
        status = 200
    body = value.get("body")
    if body is None:
        body = ""
    logging = value.get("logging")
    fixture = Fixture(status, body)
    embedding_module.get_async_httpx_client = fixture.client  # type: ignore[assignment, attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L641
    recorder = Recorder() if logging is True else None
    try:
        returned = asyncio.run(Caller().status(value, recorder))
    except json.JSONDecodeError:
        outcome: dict[str, JsonValue] = {
            "kind": "failed",
            "value": None,
            "error": "",
            "error_kind": "decode",
        }
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L665-L667
        outcome = {
            "kind": "failed",
            "value": None,
            "error": str(error),
            "error_kind": "status",
        }
    else:
        outcome = {
            "kind": "returned",
            "value": JSON.validate_python(returned),
            "error": None,
            "error_kind": None,
        }
    answer = record_value(fixture, provider, recorder)
    answer |= outcome
    return answer
