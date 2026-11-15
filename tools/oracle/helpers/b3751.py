import asyncio
import os
from typing import Literal, Protocol, TypedDict, cast

import httpx
from litellm.proxy.guardrails.guardrail_hooks.akto.akto import AktoGuardrail
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override


class AktoSettings(TypedDict, total=False):
    akto_base_url: str
    akto_api_key: str
    akto_account_id: str
    akto_vxlan_id: str
    unreachable_fallback: Literal["fail_closed", "fail_open"]
    guardrail_timeout: int


class AktoAnswer(TypedDict, total=False):
    status: int
    body: Json
    raw_body: str


class AktoCall(TypedDict, total=False):
    op: Literal["init", "payload", "response", "unreachable", "send", "fire_and_forget"]
    settings: AktoSettings
    env: dict[str, str]
    inputs: dict[str, Json]
    request_data: dict[str, Json]
    status_code: int
    include_response: bool
    answer: AktoAnswer
    guardrails: bool
    ingest_data: bool


# LiteLLM's methods take the Any-carrying inputs and bare dicts, so the instance is cast to this
class Akto(Protocol):
    akto_base_url: str
    akto_account_id: str
    akto_vxlan_id: str
    unreachable_fallback: Literal["fail_closed", "fail_open"]
    guardrail_timeout: int
    def build_akto_payload(
        self, inputs: Json, request_data: Json, *, status_code: int, include_response: bool
    ) -> Json: ...
    async def send_request(
        self, *, guardrails: bool, ingest_data: bool, payload: Json
    ) -> httpx.Response: ...
    async def fire_and_forget_request(
        self, *, guardrails: bool, ingest_data: bool, payload: Json
    ) -> None: ...
    def handle_guardrail_response(self, response: httpx.Response) -> tuple[bool, str]: ...
    def handle_unreachable(self, inputs: Json, error: Exception) -> Json: ...


class ScriptedTransport(httpx.AsyncBaseTransport):
    def __init__(self, answer: AktoAnswer) -> None:
        self.answer = answer
        self.requests: list[httpx.Request] = []

    @override
    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        status = self.answer.get("status")
        if status is None:
            status = 200
        raw = self.answer.get("raw_body")
        if raw is None:
            return httpx.Response(status, json=self.answer.get("body"), request=request)
        return httpx.Response(status, text=raw, request=request)


def build_guardrail(call: AktoCall, transport: ScriptedTransport | None = None) -> Akto:
    settings = call.get("settings")
    if settings is None:
        settings = AktoSettings()
    env = call.get("env")
    if env is not None:
        os.environ.update(env)
    fallback = settings.get("unreachable_fallback")
    if fallback is None:
        fallback = "fail_closed"
    instance = AktoGuardrail(
        akto_base_url=settings.get("akto_base_url"),
        akto_api_key=settings.get("akto_api_key"),
        akto_account_id=settings.get("akto_account_id"),
        akto_vxlan_id=settings.get("akto_vxlan_id"),
        unreachable_fallback=fallback,
        guardrail_timeout=settings.get("guardrail_timeout"),
    )
    if transport is not None:
        instance.async_handler.client = httpx.AsyncClient(transport=transport)
    return cast(Akto, instance)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/akto/akto.py#L246-L254


def inputs_of(call: AktoCall) -> dict[str, Json]:
    raw = call.get("inputs")
    if raw is None:
        return {"texts": ["hello"]}
    inputs: dict[str, Json] = {}
    for field in ("texts", "model"):
        if field in raw:
            inputs[field] = TypeAdapter(Json).validate_python(raw[field])
    return inputs


def decode_encoded(encoded: str, *, response_payload: bool) -> Json:
    outer = TypeAdapter(dict[str, Json]).validate_json(encoded)
    if response_payload and "body" not in outer:
        return outer
    inner = TypeAdapter(str).validate_python(outer["body"])
    return TypeAdapter(Json).validate_json(inner)


def normalize_payload(payload: dict[str, Json]) -> dict[str, Json]:
    normalized = TypeAdapter(dict[str, Json]).validate_python(payload)
    for field in ("requestHeaders", "responseHeaders", "tag", "metadata"):
        text = TypeAdapter(str).validate_python(payload[field])
        normalized[field] = TypeAdapter(Json).validate_json(text)
    request_payload = TypeAdapter(str).validate_python(payload["requestPayload"])
    normalized["requestPayload"] = decode_encoded(request_payload, response_payload=False)
    response_payload = TypeAdapter(str).validate_python(payload["responsePayload"])
    normalized["responsePayload"] = decode_encoded(response_payload, response_payload=True)
    time = TypeAdapter(str).validate_python(payload["time"])
    normalized["time"] = "<millis>" if time.isdigit() else time
    return normalized


def capture_request(request: httpx.Request) -> Json:
    query = TypeAdapter(dict[str, Json]).validate_python(dict(request.url.params.multi_items()))
    decoded = TypeAdapter(Json).validate_json(request.content)
    body = TypeAdapter(dict[str, Json]).validate_python(decoded)
    return {
        "path": request.url.path,
        "query": query,
        "content_type": dict(request.headers.items()).get("content-type"),
        "body": normalize_payload(body),
    }


async def deliver(
    akto: Akto, script: ScriptedTransport, payload: Json, call: AktoCall, *, fire_and_forget: bool
) -> Json:
    check = call.get("guardrails") is True
    ingest = call.get("ingest_data") is True
    if fire_and_forget:
        await akto.fire_and_forget_request(guardrails=check, ingest_data=ingest, payload=payload)
        return {"request": capture_request(script.requests[0]), "succeeded": True}
    response = await akto.send_request(guardrails=check, ingest_data=ingest, payload=payload)
    captured = capture_request(script.requests[0])
    allowed, reason = akto.handle_guardrail_response(response)
    return {"request": captured, "allowed": allowed, "reason": reason}


def payload_of(call: AktoCall, instance: Akto) -> dict[str, Json]:
    include_response = call.get("include_response") is True
    status = call.get("status_code")
    if status is None:
        status = 200
    request_data = call.get("request_data")
    if request_data is None:
        request_data = {}
    built = instance.build_akto_payload(
        inputs_of(call), request_data, status_code=status, include_response=include_response
    )
    return TypeAdapter(dict[str, Json]).validate_python(built)


def akto_init(call: AktoCall) -> Json:
    instance = build_guardrail(call)
    return {
        "base_url": instance.akto_base_url,
        "account_id": instance.akto_account_id,
        "vxlan_id": instance.akto_vxlan_id,
        "unreachable_fallback": instance.unreachable_fallback,
        "timeout": instance.guardrail_timeout,
    }


def akto_payload(call: AktoCall) -> Json:
    instance = build_guardrail(call)
    return normalize_payload(payload_of(call, instance))


def akto_response(call: AktoCall) -> Json:
    instance = build_guardrail(call)
    answer = call.get("answer")
    if answer is None:
        answer = AktoAnswer()
    status = answer.get("status")
    if status is None:
        status = 200
    raw = answer.get("raw_body")
    request = httpx.Request("POST", "http://akto.invalid/api/http-proxy")
    if raw is None:
        response = httpx.Response(status, json=answer.get("body"), request=request)
    else:
        response = httpx.Response(status, text=raw, request=request)
    allowed, reason = instance.handle_guardrail_response(response)
    return {"allowed": allowed, "reason": reason}


def akto_unreachable(call: AktoCall) -> Json:
    instance = build_guardrail(call)
    error = httpx.ConnectError("the Akto service is unreachable")
    returned = instance.handle_unreachable(inputs_of(call), error)
    return {"passthrough": TypeAdapter(Json).validate_python(returned)}


def akto_send(call: AktoCall) -> Json:
    answer = call.get("answer")
    if answer is None:
        answer = AktoAnswer()
    transport = ScriptedTransport(answer)
    instance = build_guardrail(call, transport)
    payload = payload_of(call, instance)
    fire_and_forget = call.get("op") == "fire_and_forget"
    return asyncio.run(
        deliver(instance, transport, payload, call, fire_and_forget=fire_and_forget)
    )


def run(input_value: AktoCall) -> Json:
    operation = input_value.get("op")
    if operation == "init":
        return akto_init(input_value)
    if operation == "payload":
        return akto_payload(input_value)
    if operation == "response":
        return akto_response(input_value)
    if operation == "unreachable":
        return akto_unreachable(input_value)
    if operation in ("send", "fire_and_forget"):
        return akto_send(input_value)
    raise ValueError(f"unknown operation: {operation}")
