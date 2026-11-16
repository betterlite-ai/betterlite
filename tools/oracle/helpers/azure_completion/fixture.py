from datetime import UTC, datetime
from typing import Final

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue
from typing_extensions import override

from .types import (
    API_BASE,
    API_VERSION,
    JSON,
    MODEL,
    ORIGINAL,
    POST_ARGS,
    PRE_ARGS,
    Answer,
    CallCase,
    PreArgs,
)

BODY: Final = (
    '{"id":"cmpl-1","object":"text_completion","created":1700000000,'
    '"model":"gpt-35-turbo-instruct",'
    '"choices":[{"text":"Hello!","index":0,"logprobs":null,"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}'
)


def call_view(
    params: dict[str, JsonValue],
    headers: dict[str, JsonValue] | None,
    api_base: JsonValue,
    api_version: str | None,
    extra: dict[str, JsonValue],
) -> JsonValue:
    return {
        "input": params.get("input"),
        "headers": headers,
        "api_base": api_base,
        "api_version": api_version,
        **extra,
    }


def pre_api_base(additional: PreArgs) -> JsonValue:
    if additional.acompletion:
        return None
    return JSON.validate_python(additional.api_base)


def pre_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = PRE_ARGS.validate_python(params.get("additional_args"))
    version = None if additional.acompletion else additional.api_version
    flags: dict[str, JsonValue] = {
        "acompletion": additional.acompletion,
        "complete_input_dict": additional.complete_input_dict,
    }
    return call_view(params, additional.headers, pre_api_base(additional), version, flags)


def post_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = POST_ARGS.validate_python(params.get("additional_args"))
    original = ORIGINAL.validate_python(params.get("original_response"))
    origin: dict[str, JsonValue] = {"original_id": original["id"]}
    return call_view(params, additional.headers, additional.api_base, additional.api_version, origin)


class Recorder(Logging):
    def __init__(self) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
            model=MODEL,
            messages=None,
            stream=False,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre: list[JsonValue] = []
        self.post: list[JsonValue] = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.pre.append(pre_view(params))

    @override
    def post_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.post.append(post_view(params))


def response_for(answer: Answer, request: httpx.Request) -> httpx.Response:
    status = answer.get("status")
    if status is None:
        status = 200
    headers = answer.get("headers")
    if "headers" not in answer:
        headers = {"content-type": "application/json"}
    events = answer.get("events")
    if events is not None:
        return httpx.Response(
            status, headers=headers, content="".join(events).encode(), request=request
        )
    body = answer.get("body")
    if body is None:
        body = BODY
    return httpx.Response(status, headers=headers, content=body.encode(), request=request)


class Fixture:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.requests: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append({"data": JSON.validate_json(request.content)})
        return response_for(self.answer, request)


def source_key(case: CallCase) -> str | None:
    if "client_api_key" in case:
        return case["client_api_key"]
    return case.get("api_key")


def client_for(case: CallCase, fixture: Fixture) -> AzureOpenAI | AsyncAzureOpenAI:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    version = case.get("api_version")
    endpoint = case.get("api_base")
    requested = case.get("client")
    if "client" not in case:
        requested = "async" if case.get("acompletion") else "sync"
    if requested == "async":
        return AsyncAzureOpenAI(
            api_key=key,
            azure_ad_token=token,
            api_version=API_VERSION if version is None else version,
            azure_endpoint=API_BASE if endpoint is None else endpoint,
            max_retries=0,
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(fixture)),
        )
    return AzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=API_VERSION if version is None else version,
        azure_endpoint=API_BASE if endpoint is None else endpoint,
        max_retries=0,
        http_client=httpx.Client(transport=httpx.MockTransport(fixture)),
    )
