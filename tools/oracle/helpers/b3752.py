import asyncio
import json
from typing import Literal, TypedDict

import httpx
from litellm.exceptions import GuardrailRaisedException
from litellm.proxy.guardrails.guardrail_hooks.alice.alice import AliceGuardrail
from litellm.proxy.guardrails.guardrail_hooks.conduct.conduct import request_payload
from pydantic import JsonValue, TypeAdapter

TEST_API_BASE = "https://alice.test"


class ScriptedAnswer(TypedDict):
    status: int
    body: JsonValue
    raw: str | None


class AliceApply(TypedDict):
    function: Literal["alice_apply"]
    api_key: str | None
    unreachable_fallback: Literal["fail_closed", "fail_open"]
    texts: list[str] | None
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]
    answer: ScriptedAnswer | None
    failure: Literal["connect", "timeout"] | None


class ConductRequestPayload(TypedDict):
    function: Literal["conduct_request_payload"]
    texts: list[str] | None
    structured_message_text: str | None
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]


Input = AliceApply | ConductRequestPayload


class Handler:
    def __init__(
        self,
        answer: ScriptedAnswer | None,
        failure: Literal["connect", "timeout"] | None,
    ) -> None:
        self.answer = answer
        self.failure = failure
        self.requests: list[dict[str, JsonValue]] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        if self.failure == "connect":
            raise httpx.ConnectError("the mock refuses the connection", request=request)
        if self.failure == "timeout":
            raise httpx.ReadTimeout("the mock never answers", request=request)
        self.requests.append(capture(request))
        if self.answer is None:
            raise ValueError("a scripted answer or a failure is required")
        if self.answer["raw"] is not None:
            return httpx.Response(self.answer["status"], content=self.answer["raw"])
        return httpx.Response(self.answer["status"], json=self.answer["body"])


def capture(request: httpx.Request) -> dict[str, JsonValue]:
    return {
        "path": request.url.path,
        "af_api_key": request.headers["af-api-key"],
        "content_type": request.headers["content-type"],
        "body": TypeAdapter(JsonValue).validate_json(request.content),
    }


def alice_result(
    texts: list[str],
    handler: Handler,
    *,
    raised: bool,
    status: int | None,
    blocked_content: bool | None,
) -> dict[str, JsonValue]:
    result: dict[str, JsonValue] = {}
    result["texts"] = TypeAdapter(JsonValue).validate_python(texts)
    result["requests"] = TypeAdapter(JsonValue).validate_python(handler.requests)
    result["raised"] = raised
    result["status"] = status
    result["blocked_content"] = blocked_content
    return result


def alice_apply(args: AliceApply) -> JsonValue:
    handler = Handler(args["answer"], args["failure"])
    guardrail = AliceGuardrail(
        api_key=args["api_key"],
        api_base=TEST_API_BASE,
        unreachable_fallback=args["unreachable_fallback"],
    )
    guardrail.async_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(handler)
    )
    texts = [] if args["texts"] is None else list(args["texts"])
    kind = args["input_type"]
    try:
        asyncio.run(
            guardrail.apply_guardrail({"texts": texts}, args["request_data"], kind)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536-L4547
        )
    except GuardrailRaisedException as error:
        return alice_result(
            [],
            handler,
            raised=True,
            status=error.status_code,
            blocked_content=error.blocked_content,
        )
    return alice_result(
        texts, handler, raised=False, status=None, blocked_content=None
    )


def conduct_inputs(args: ConductRequestPayload) -> dict[str, JsonValue]:
    inputs: dict[str, JsonValue] = {}
    if args["structured_message_text"] is not None:
        message: dict[str, JsonValue] = {
            "role": "user",
            "content": args["structured_message_text"],
        }
        messages: list[JsonValue] = [message]
        inputs["structured_messages"] = messages
    elif args["texts"] is not None:
        inputs["texts"] = TypeAdapter(JsonValue).validate_python(args["texts"])
    return inputs


def conduct_request_payload(args: ConductRequestPayload) -> JsonValue:
    kind = args["input_type"]
    payload = request_payload(conduct_inputs(args), args["request_data"], kind)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536-L4547
    return {"payload": TypeAdapter(JsonValue).validate_json(json.dumps(payload, default=dict))}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "alice_apply":
        return alice_apply(input_value)
    return conduct_request_payload(input_value)
