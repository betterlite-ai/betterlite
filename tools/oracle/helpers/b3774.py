import asyncio
import json
import uuid
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypedDict

import httpx
from fastapi import HTTPException
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.guardrails.guardrail_hooks.headroom.headroom import HeadroomGuardrail
from pydantic import JsonValue, TypeAdapter

API_BASE: Final = "https://headroom.test"
TRANSPORT_DETAIL: Final = "<transport-error>"
JSON: Final = TypeAdapter(JsonValue)


class RequestState(TypedDict, total=False):
    litellm_call_id: str


STATE: Final = TypeAdapter(RequestState)


class ServiceAnswer(TypedDict):
    status: int
    headers: dict[str, str]
    body: NotRequired[JsonValue]
    events: NotRequired[list[str]]


class CompressionInput(TypedDict):
    inputs: dict[str, JsonValue]
    request_data: NotRequired[dict[str, JsonValue]]
    input_type: NotRequired[Literal["request", "response"]]
    api_key: NotRequired[str]
    model: NotRequired[str]
    unreachable_fallback: NotRequired[Literal["fail_closed", "fail_open"]]
    timeout: NotRequired[float]
    logging_call_id: NotRequired[str]
    answer: NotRequired[ServiceAnswer]
    silent: NotRequired[bool]


class Service:
    def __init__(self, request_input: CompressionInput) -> None:
        self.calls: list[dict[str, JsonValue]] = []
        self.status = 200
        self.headers: dict[str, str] = {}
        self.body = b""
        self.silent = bool(request_input.get("silent"))
        answer = request_input.get("answer")
        self.unreachable = answer is None and not self.silent
        if answer is None:
            return
        self.status = answer["status"]
        self.headers = answer["headers"]
        if "body" in answer:
            encoded = json.dumps(
                answer["body"], sort_keys=True, separators=(",", ":"), ensure_ascii=False
            )
            self.body = encoded.encode()
        if "events" in answer:
            self.body = "".join(f"data: {item}\n\n" for item in answer["events"]).encode()

    def __call__(self, request: httpx.Request) -> httpx.Response:
        if self.unreachable:
            raise httpx.ConnectError("the compression service is unreachable", request=request)
        if self.silent:
            raise httpx.ReadTimeout("the compression service timed out", request=request)
        self.calls.append(
            {
                "method": request.method,
                "path": request.url.path,
                "authorization_present": "authorization" in request.headers,
                "content_type": request.headers.get("content-type"),
                "body": JSON.validate_json(request.content),
            }
        )
        return httpx.Response(self.status, headers=self.headers, content=self.body)


def state_call_id(data: dict[str, JsonValue]) -> str | None:
    supplied = STATE.validate_python(data).get("litellm_call_id")
    return supplied if supplied else None


def read_issued(guardrail: HeadroomGuardrail, call_id: str | None) -> JsonValue:
    if call_id is None:
        return None
    issued = guardrail._issued_hashes_by_call_id  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L867
    if call_id not in issued:
        return None
    hashes, _expiry = issued[call_id]
    return JSON.validate_python(sorted(hashes))


def is_uuid(call_id: str | None) -> bool:
    if call_id is None:
        return False
    try:
        uuid.UUID(call_id)
    except ValueError:
        return False
    return True


async def invoke(
    guardrail: HeadroomGuardrail,
    inputs: dict[str, JsonValue],
    request_data: dict[str, JsonValue],
    input_type: Literal["request", "response"],
    logging_obj: Logging | None,
) -> JsonValue:
    return JSON.validate_python(
        await guardrail.apply_guardrail(inputs, request_data, input_type, logging_obj)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580
    )


def run(request_input: CompressionInput) -> JsonValue:
    service = Service(request_input)
    guardrail = HeadroomGuardrail(
        api_base=API_BASE,
        api_key=request_input.get("api_key"),
        model=request_input.get("model"),
        guardrail_name="headroom",
        unreachable_fallback=request_input.get("unreachable_fallback"),
        timeout=request_input.get("timeout"),
        ccr_retrieval=True,
    )
    guardrail.async_handler = AsyncHTTPHandler(transport=httpx.MockTransport(service))
    request_data: dict[str, JsonValue] = {}
    given = request_input.get("request_data")
    if given is not None:
        request_data = given
    logging_id = request_input.get("logging_call_id")
    logging_obj: Logging | None = None
    if logging_id:
        logging_obj = Logging(
            model="gpt-4o",
            messages=None,
            stream=False,
            call_type=None,
            start_time=datetime(2024, 1, 1, tzinfo=UTC),
            litellm_call_id=logging_id,
            function_id="headroom",
        )
    requested = request_input.get("input_type")
    kind: Literal["request", "response"] = "request" if requested is None else requested
    returned: JsonValue = None
    raised: JsonValue = None
    try:
        returned = asyncio.run(
            invoke(guardrail, request_input["inputs"], request_data, kind, logging_obj)
        )
    except HTTPException as error:
        detail = TypeAdapter(dict[str, JsonValue]).validate_json(json.dumps(error.detail))
        if "detail" in detail:
            detail["detail"] = TRANSPORT_DETAIL
        raised = {"status": error.status_code, "detail": detail}
    supplied = logging_id if logging_id else None
    before = request_input.get("request_data")
    if supplied is None and before is not None:
        supplied = state_call_id(before)
    generated = state_call_id(request_data)
    call_id = generated if supplied is None else supplied
    identity_is_new = supplied is None and generated is not None
    answer: JsonValue = {
        "inputs": returned,
        "raised": raised,
        "request_data": request_data,
        "calls": JSON.validate_python(service.calls),
        "call_id_is_uuid": identity_is_new and is_uuid(call_id),
        "issued": read_issued(guardrail, call_id),
    }
    return answer
