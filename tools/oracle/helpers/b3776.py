import asyncio
from collections import deque
from functools import partial
from typing import Final, Literal, NotRequired, Required, TypedDict
from uuid import UUID

import httpx
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer import hiddenlayer as hl
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer import initialize_guardrail
from litellm.types.guardrails import LitellmParams
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: Final = TypeAdapter(JsonValue)
VIEW: Final = TypeAdapter(dict[str, JsonValue])
TEXT: Final = TypeAdapter(str)
LOCAL_BASE: Final = "http://127.0.0.1:1"
FIXED_ID: Final = "11111111-1111-1111-1111-111111111111"


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
    inputs: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    input_type: Literal["request", "response"]


class Captured(TypedDict):
    path: str
    headers: dict[str, str]
    authorization: str | None
    body: JsonValue


class Raised(TypedDict):
    kind: Literal["http", "failed"]
    status: NotRequired[int]
    detail: NotRequired[str]


class Applied(TypedDict):
    texts: JsonValue
    structured_messages: JsonValue
    tool_calls: JsonValue


class Outcome(TypedDict):
    requests: list[Captured]
    raised: Raised | None
    result: Applied | None
    roundtrip: JsonValue


class Dispatched(TypedDict):
    supported_event_hooks: list[str] | None
    raised: None


class Recorder:
    def __init__(self, tokens: list[str] | None, answers: list[Reply] | None) -> None:
        self.tokens: list[str] = [] if tokens is None else tokens
        self.answers: deque[Reply] = deque([] if answers is None else answers)
        self.seen: list[Captured] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        authorization = request.headers["authorization"] if "authorization" in request.headers else None
        for index, token in enumerate(self.tokens):
            if authorization == f"Bearer {token}":
                authorization = f"bearer:{index}"
                break
        kept = {
            name.lower(): value
            for name, value in request.headers.items()
            if name.lower().startswith("hl-") or name.lower() == "content-type"
        }
        self.seen.append(
            Captured(
                path=request.url.path,
                headers=kept,
                authorization=authorization,
                body=JSON.validate_json(request.content),
            )
        )
        if not self.answers:
            return httpx.Response(500, content=b"{}")
        reply = self.answers.popleft()
        return httpx.Response(reply["status"], headers=reply.get("headers"), json=reply["body"])


class Stub(Logging):
    def __init__(self, view: dict[str, JsonValue]) -> None:
        model = view.get("model")
        details = view.get("model_call_details")
        empty: dict[str, JsonValue] = {}
        self.model = "" if model is None else TEXT.validate_python(model)
        self.model_call_details = empty if details is None else VIEW.validate_python(details)


def build_params(input_value: Input, base: str) -> LitellmParams:
    params = dict(input_value["params"])
    params["api_base"] = base
    params["auth_url"] = base
    return LitellmParams(**params)


def build_guardrail(input_value: Input) -> hl.HiddenlayerGuardrail | hl.HiddenlayerGuardrailV2:
    params = build_params(input_value, LOCAL_BASE)
    name = input_value["guardrail_name"]
    return initialize_guardrail(params, {"guardrail_name": name, "litellm_params": params})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1291-L1294


def messages_of(value: JsonValue) -> JsonValue:
    if value is None:
        return None
    match value:
        case list(messages):
            return messages
        case dict(fields):
            return fields.get("messages")
        case str() | bool() | int() | float():
            return value


def roundtrip_of(request_data: dict[str, JsonValue]) -> JsonValue:
    proxy = request_data.get("proxy_server_request")
    if proxy is None:
        return None
    headers = VIEW.validate_python(proxy).get("headers")
    if headers is None:
        return None
    return VIEW.validate_python(headers).get("hl-roundtrip-id")


def apply(input_value: Input, recorder: Recorder) -> JsonValue:
    guardrail = build_guardrail(input_value)
    guardrail._http_client = AsyncHTTPHandler(transport=httpx.MockTransport(recorder.handle))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/hiddenlayer/hiddenlayer.py#L471
    agent: CustomGuardrail = guardrail
    if input_value.get("saas"):
        guardrail.jwt_token = recorder.tokens[0]
        guardrail.refresh_jwt_func = partial(next, iter(recorder.tokens[1:]), "")
    hl.uuid4 = partial(UUID, FIXED_ID)  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/hiddenlayer/hiddenlayer.py#L7
    request_data: dict[str, JsonValue] = {}
    if "request_data" in input_value:
        request_data = dict(input_value["request_data"])
    inputs: dict[str, JsonValue] = {}
    if "inputs" in input_value:
        inputs = dict(input_value["inputs"])
    kind: Literal["request", "response"] = "request"
    if "input_type" in input_value:
        kind = input_value["input_type"]
    logging = request_data.get("litellm_logging_obj")
    stub = None if logging is None else Stub(VIEW.validate_python(logging))
    raised: Raised | None = None
    try:
        asyncio.run(agent.apply_guardrail(inputs, request_data, kind, stub))  # type: ignore[arg-type,misc]  # pyright: ignore[reportArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536-L4547
    except HTTPException as error:
        raised = Raised(kind="http", status=error.status_code, detail=error.detail)
    except httpx.HTTPStatusError:
        raised = Raised(kind="failed")
    result: Applied | None = None
    if raised is None:
        result = Applied(
            texts=inputs.get("texts"),
            structured_messages=messages_of(inputs.get("structured_messages")),
            tool_calls=inputs.get("tool_calls"),
        )
    return JSON.validate_python(
        Outcome(
            requests=recorder.seen,
            raised=raised,
            result=result,
            roundtrip=roundtrip_of(request_data),
        )
    )


def dispatch(input_value: Input) -> JsonValue:
    guardrail = build_guardrail(input_value)
    hooks = guardrail.get_supported_event_hooks()
    names = None if hooks is None else [hook.name for hook in hooks]
    return JSON.validate_python(Dispatched(supported_event_hooks=names, raised=None))


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "dispatch":
        return dispatch(input_value)
    if function != "apply":
        raise ValueError(f"unsupported function {function}")
    return apply(input_value, Recorder(input_value.get("tokens"), input_value.get("answers")))
