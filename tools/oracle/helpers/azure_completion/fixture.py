from datetime import UTC, datetime
from typing import Final

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue
from typing_extensions import override

from .types import (
    JSON,
    MODEL,
    ORIGINAL,
    POST_ARGS,
    PRE_ARGS,
    Answer,
    PreArgs,
)

BODY: Final = (
    '{"id":"cmpl-1","object":"text_completion","created":1700000000,'
    '"model":"gpt-35-turbo-instruct",'
    '"choices":[{"text":"Hello!","index":0,"logprobs":null,"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}'
)


class Recorder(Logging):
    def __init__(self) -> None:
        super().__init__(
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
        additional = PRE_ARGS.validate_python(params.get("additional_args"))
        self.pre.append(
            {
                "input": params.get("input"),
                "headers": additional.headers,
                "api_base": pre_api_base(additional),
                "api_version": None if additional.acompletion else additional.api_version,
                "acompletion": additional.acompletion,
                "complete_input_dict": additional.complete_input_dict,
            }
        )

    @override
    def post_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        additional = POST_ARGS.validate_python(params.get("additional_args"))
        original = params.get("original_response")
        self.post.append(
            {
                "input": params.get("input"),
                "headers": additional.headers,
                "api_base": additional.api_base,
                "api_version": additional.api_version,
                "original_id": ORIGINAL.validate_python(original)["id"],
            }
        )


class Fixture:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.requests: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append({"data": JSON.validate_json(request.content)})
        return response_for(self.answer, request)


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


def pre_api_base(additional: PreArgs) -> JsonValue:
    if additional.acompletion:
        return None
    return JSON.validate_python(additional.api_base)
