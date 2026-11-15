import asyncio
from collections import deque
from typing import Final, Literal, NotRequired, Required, TypedDict
from uuid import UUID

import httpx
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.llms.custom_httpx.http_handler import get_async_httpx_client, httpxSpecialProvider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer import hiddenlayer as hl
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer import initialize_guardrail
from litellm.types.guardrails import LitellmParams
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: Final = TypeAdapter(JsonValue)
LOCAL_BASE: Final = "http://127.0.0.1:1"


class Reply(TypedDict):
    status: int
    body: JsonValue
    headers: NotRequired[dict[str, str]]


class Input(TypedDict, total=False):
    function: Required[str]
    params: Required[dict[str, JsonValue]]
    guardrail_name: Required[str]
    saas: bool
    tokens: list[str]
    answers: list[Reply]
    inputs: GenericGuardrailAPIInputs
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]


class Recorder:
    def __init__(self, tokens: list[str], answers: list[Reply]) -> None:
        self.tokens: list[str] = tokens
        self.answers: deque[Reply] = deque(answers)
        self.seen: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        authorization = request.headers.get("authorization")
        for index, token in enumerate(self.tokens):
            if authorization is not None and authorization == f"Bearer {token}":
                authorization = f"bearer:{index}"
                break
        self.seen.append(
            JSON.validate_python(
                {
                    "path": request.url.path,
                    "headers": {
                        name.lower(): value
                        for name, value in request.headers.items()
                        if name.lower().startswith("hl-") or name.lower() == "content-type"
                    },
                    "authorization": authorization,
                    "body": JSON.validate_json(request.content),
                }
            )
        )
        if not self.answers:
            return httpx.Response(500, json={})
        reply = self.answers.popleft()
        return httpx.Response(reply["status"], headers=reply.get("headers"), json=reply["body"])


class Stub(Logging):
    def __init__(self, view: dict[str, JsonValue]) -> None:
        model = view.get("model")
        details = view.get("model_call_details")
        self.model = "" if model is None else TypeAdapter(str).validate_python(model)
        self.model_call_details = (
            {} if details is None else TypeAdapter(dict[str, JsonValue]).validate_python(details)
        )


def build_params(input_value: Input, base: str) -> LitellmParams:
    params = dict(input_value["params"])
    params["api_base"] = base
    params["auth_url"] = base
    return LitellmParams(**params)


def messages_of(value: JsonValue) -> JsonValue:
    if value is None or isinstance(value, list):
        return value
    messages = TypeAdapter(dict[str, JsonValue]).validate_python(value).get("messages")
    return [] if messages is None else messages


def apply(input_value: Input, recorder: Recorder) -> JsonValue:
    handler = get_async_httpx_client(llm_provider=httpxSpecialProvider.GuardrailCallback)
    handler.client = httpx.AsyncClient(transport=httpx.MockTransport(recorder.handle))
    params = build_params(input_value, LOCAL_BASE)
    guardrail = initialize_guardrail(
        params, {"guardrail_name": input_value["guardrail_name"], "litellm_params": params}
    )
    if input_value.get("saas"):
        guardrail.jwt_token = recorder.tokens[0]
        refresh = deque(recorder.tokens[1:])
        guardrail.refresh_jwt_func = lambda: refresh.popleft() if refresh else ""
    hl.uuid4 = lambda: UUID("11111111-1111-1111-1111-111111111111")
    request_data = dict(input_value["request_data"])
    logging = request_data.get("litellm_logging_obj")
    inputs = input_value["inputs"]
    agent: CustomGuardrail = guardrail
    raised: JsonValue = None
    try:
        asyncio.run(
            agent.apply_guardrail(
                inputs,
                request_data,
                input_value["input_type"],
                None
                if logging is None
                else Stub(TypeAdapter(dict[str, JsonValue]).validate_python(logging)),
            )
        )
    except HTTPException as error:
        raised = JSON.validate_python(
            {"kind": "http", "status": error.status_code, "detail": error.detail}
        )
    except httpx.HTTPStatusError:
        raised = JSON.validate_python({"kind": "failed"})
    after = JSON.validate_python(inputs)
    result: JsonValue = None
    if raised is None:
        result = JSON.validate_python(
            {
                "texts": after.get("texts"),
                "structured_messages": messages_of(after.get("structured_messages")),
                "tool_calls": after.get("tool_calls"),
            }
        )
    roundtrip: JsonValue = None
    proxy = request_data.get("proxy_server_request")
    if proxy is not None:
        headers = TypeAdapter(dict[str, JsonValue]).validate_python(proxy).get("headers")
        if headers is not None:
            roundtrip = TypeAdapter(dict[str, JsonValue]).validate_python(headers).get(
                "hl-roundtrip-id"
            )
    return JSON.validate_python(
        {"requests": recorder.seen, "raised": raised, "result": result, "roundtrip": roundtrip}
    )


def dispatch(input_value: Input) -> JsonValue:
    params = build_params(input_value, LOCAL_BASE)
    guardrail = initialize_guardrail(
        params, {"guardrail_name": input_value["guardrail_name"], "litellm_params": params}
    )
    hooks = guardrail.get_supported_event_hooks()
    return JSON.validate_python(
        {
            "supported_event_hooks": None if hooks is None else [hook.value for hook in hooks],
            "raised": None,
        }
    )


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "dispatch":
        return dispatch(input_value)
    if function != "apply":
        raise ValueError(f"unsupported function {function}")
    return apply(input_value, Recorder(input_value.get("tokens", []), input_value.get("answers", [])))
