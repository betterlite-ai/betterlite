import asyncio
from typing import Final, Literal, NotRequired, Protocol, TypedDict

from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.tool_permission import ToolPermissionGuardrail
from litellm.types.utils import CallTypesLiteral
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

GUARDRAIL_NAME: Final = "tool-permissions"


class Input(TypedDict):
    rules: list[dict[str, JsonValue]]
    default_action: Literal["allow", "deny"]
    on_disallowed_action: Literal["block", "rewrite"]
    event_hook: Literal["pre_call", "post_call"]
    default_on: bool
    violation_message_template: NotRequired[str]
    data: dict[str, JsonValue]


class Detail(TypedDict):
    error: str
    detection_message: str


class Blocked(TypedDict):
    status: int
    detail: Detail


class Forwarded(TypedDict, total=False):
    tools: list[JsonValue] | None
    functions: list[JsonValue] | None
    tool_choice: JsonValue
    function_call: JsonValue


class Bucket(TypedDict, total=False):
    applied_guardrails: list[str]


class Buckets(TypedDict, total=False):
    metadata: Bucket | None
    litellm_metadata: Bucket | None


class Output(TypedDict):
    blocked: Blocked | None
    request: Forwarded
    applied_guardrails: list[str] | None


class PreCallHook(Protocol):
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: dict[str, JsonValue],
        call_type: CallTypesLiteral,
    ) -> Exception | str | dict[str, JsonValue] | None: ...


DETAIL: Final = TypeAdapter(Detail)
FORWARDED: Final = TypeAdapter(Forwarded)
BUCKETS: Final = TypeAdapter(Buckets)


async def invoke(hook: PreCallHook, data: dict[str, JsonValue]) -> None:
    await hook.async_pre_call_hook(UserAPIKeyAuth(), DualCache(), data, "completion")


def run(input_value: Input) -> Output:
    guardrail = ToolPermissionGuardrail(
        rules=input_value["rules"],
        default_action=input_value["default_action"],
        on_disallowed_action=input_value["on_disallowed_action"],
        guardrail_name=GUARDRAIL_NAME,
        event_hook=input_value["event_hook"],
        default_on=input_value["default_on"],
        violation_message_template=input_value.get("violation_message_template"),
    )
    data = input_value["data"]
    blocked: Blocked | None = None
    try:
        asyncio.run(invoke(guardrail, data))
    except HTTPException as error:
        blocked = {"status": error.status_code, "detail": DETAIL.validate_python(error.detail)}
    buckets = BUCKETS.validate_python(data)
    bucket = (
        buckets.get("litellm_metadata")
        if "litellm_metadata" in data
        else buckets.get("metadata")
    )
    applied: list[str] | None = None
    if bucket is not None and "applied_guardrails" in bucket:
        applied = bucket["applied_guardrails"]
    return {
        "blocked": blocked,
        "request": FORWARDED.validate_python(data),
        "applied_guardrails": applied,
    }
