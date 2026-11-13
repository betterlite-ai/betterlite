import asyncio
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.caching.dual_cache import DualCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.lakera_ai_v2 import (
    LakeraAIGuardrail,
    humanize_lakera_block_reasons,
)
from litellm.types.proxy.guardrails.guardrail_hooks.lakera_ai_v2 import (
    LakeraAIBreakdownItem,
)
from pydantic import JsonValue, TypeAdapter

REPLY: dict[str, JsonValue] = {"flagged": False, "payload": [], "breakdown": []}


class HumanizeInput(TypedDict):
    function: Literal["humanize_lakera_block_reasons"]
    breakdown: list[LakeraAIBreakdownItem] | None


class ValidateInput(TypedDict):
    function: Literal["validate_advisory_config"]
    on_flagged: Literal["block", "monitor", "inject_system_message"] | None
    advisory_system_message: str | None
    payload: bool | None
    breakdown: bool | None


class InspectionInput(TypedDict):
    function: Literal["build_lakera_inspection_messages"]
    data: dict[str, JsonValue]


Input: TypeAlias = HumanizeInput | ValidateInput | InspectionInput


# The guardrail's own hook types its return as a bare dict, so this names what the oracle reads
class Hook(Protocol):
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: dict[str, JsonValue],
        call_type: Literal["completion"],
    ) -> None | str | Exception | dict[str, JsonValue]: ...


def as_hook(guardrail: LakeraAIGuardrail) -> Hook:
    hook: Hook = guardrail
    return hook


class Recorder:
    def __init__(self) -> None:
        self.messages: JsonValue = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        body = TypeAdapter(dict[str, JsonValue]).validate_json(request.content)
        self.messages = body["messages"]
        return httpx.Response(200, json=REPLY)


def validate(fields: ValidateInput) -> JsonValue:
    try:
        _guardrail = LakeraAIGuardrail(
            api_key="test",
            on_flagged=fields["on_flagged"],
            advisory_system_message=fields["advisory_system_message"],
            payload=fields["payload"],
            breakdown=fields["breakdown"],
        )
    except ValueError as error:
        return {"ok": False, "error": str(error)}
    return {"ok": True}


def inspection_messages(data: dict[str, JsonValue]) -> JsonValue:
    recorder = Recorder()
    guardrail = LakeraAIGuardrail(api_key="test", api_base="http://lakera.test")
    handler = AsyncHTTPHandler()
    handler.client = httpx.AsyncClient(transport=httpx.MockTransport(recorder.handle))
    guardrail.async_handler = handler
    hook = as_hook(guardrail)
    _answer = asyncio.run(
        hook.async_pre_call_hook(UserAPIKeyAuth(), DualCache(), data, "completion")
    )
    return {"messages": recorder.messages}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "humanize_lakera_block_reasons":
        reason = humanize_lakera_block_reasons(input_value["breakdown"])
        return {"reason": reason}
    if input_value["function"] == "validate_advisory_config":
        return validate(input_value)
    return inspection_messages(input_value["data"])
