import asyncio
import os
from typing import Final, Literal, Protocol, TypedDict, cast

import httpx
from fastapi import HTTPException
from litellm.caching.dual_cache import DualCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.repelloai import repelloai
from litellm.proxy.guardrails.guardrail_hooks.repelloai.repelloai import RepelloAIGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import CallTypesLiteral
from pydantic import JsonValue, TypeAdapter

BODY_ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

API_KEY_HEADER: Final = "x-api-key"
CLEARED_ENVIRONMENT = ("ARGUS_API_KEY", "REPELLOAI_API_KEY", "REPELLOAI_API_BASE")


class Answer(TypedDict):
    status: int
    body: dict[str, JsonValue]


class RequestRecord(TypedDict):
    path: str
    api_key_present: bool
    body: JsonValue


class Caught(TypedDict):
    status: int | None
    detail: JsonValue


class GuardrailRecord(TypedDict):
    guardrail_status: str


class HookData(TypedDict, total=False):
    messages: list[dict[str, str]]
    metadata: dict[str, JsonValue]
    litellm_metadata: dict[str, JsonValue]


class Output(TypedDict):
    created: bool
    api_base: str | None
    asset_id: str | None
    key_source: str | None
    requests: list[RequestRecord]
    applied: list[str]
    status: str | None
    caught: Caught | None


class Guardrail(Protocol):
    async_handler: AsyncHTTPHandler
    api_base: str
    asset_id: str | None
    repelloai_api_key: str

    async def async_pre_call_hook(
        self,
        *,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: HookData,
        call_type: CallTypesLiteral,
    ) -> Exception | str | dict[str, JsonValue] | None: ...


class Input(TypedDict):
    mode: Literal["init", "pre_call"]
    env: dict[str, str]
    api_key: str | None
    api_base: str | None
    asset_id: str | None
    unreachable_fallback: str | None
    data: HookData
    answer: Answer | None


class Script:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.calls: list[RequestRecord] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "path": request.url.path,
                "api_key_present": API_KEY_HEADER in request.headers,
                "body": BODY_ADAPTER.validate_json(request.content),
            }
        )
        return httpx.Response(self.answer["status"], json=self.answer["body"], request=request)


def build_guardrail(call: Input) -> Guardrail:
    for name in CLEARED_ENVIRONMENT:
        os.environ.pop(name, None)
    os.environ.update(call["env"])
    fallback: Literal["fail_closed", "fail_open"] = (
        "fail_open" if call["unreachable_fallback"] == "fail_open" else "fail_closed"
    )
    return cast(Guardrail, RepelloAIGuardrail(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/repelloai/repelloai.py#L142-L182

        guardrail_name="repello",
        api_key=call["api_key"],
        api_base=call["api_base"],
        asset_id=call["asset_id"],
        unreachable_fallback=fallback,
        event_hook=GuardrailEventHooks.pre_call,
        default_on=True,
    ))


def key_source(call: Input, key: str) -> str:
    if call["api_key"] == key:
        return "explicit"
    if call["env"].get("ARGUS_API_KEY") == key:
        return "argus"
    return "repelloai"


def hook_facts(data: HookData) -> tuple[list[str], str | None]:
    bucket = data.get("litellm_metadata") or data.get("metadata")
    if bucket is None:
        return [], None
    applied = bucket.get("applied_guardrails")
    names = TypeAdapter(list[str]).validate_python(applied) if applied is not None else []
    recorded = TypeAdapter(list[GuardrailRecord]).validate_python(
        bucket["standard_logging_guardrail_information"]
    )
    return names, recorded[-1]["guardrail_status"] if recorded else None


def empty_output() -> Output:
    return {
        "created": False,
        "api_base": None,
        "asset_id": None,
        "key_source": None,
        "requests": [],
        "applied": [],
        "status": None,
        "caught": None,
    }


def created_output(call: Input, guardrail: Guardrail) -> Output:
    output = empty_output()
    output["created"] = True
    output["api_base"] = guardrail.api_base
    output["asset_id"] = guardrail.asset_id
    output["key_source"] = key_source(call, guardrail.repelloai_api_key)
    return output


def init_output(call: Input) -> Output:
    try:
        guardrail = build_guardrail(call)
    except (repelloai.RepelloAIGuardrailMissingSecrets, ValueError):
        return empty_output()
    return created_output(call, guardrail)


def hook_output(call: Input) -> Output:
    guardrail = build_guardrail(call)
    answer = call["answer"]
    if answer is None:
        raise ValueError("the case has no answer")
    script = Script(answer)
    guardrail.async_handler = AsyncHTTPHandler(transport=httpx.MockTransport(script))
    data = call["data"]
    caught: Caught | None = None
    try:
        asyncio.run(
            guardrail.async_pre_call_hook(
                user_api_key_dict=UserAPIKeyAuth(),
                cache=DualCache(),
                data=data,
                call_type="completion",
            )
        )
    except HTTPException as error:
        caught = {"status": error.status_code, "detail": error.detail}
    output = created_output(call, guardrail)
    applied, status = hook_facts(data)
    output["requests"] = script.calls
    output["applied"] = applied
    output["status"] = status
    output["caught"] = caught
    return output


def run(input_value: Input) -> Output:
    if input_value["mode"] == "init":
        return init_output(input_value)
    return hook_output(input_value)
