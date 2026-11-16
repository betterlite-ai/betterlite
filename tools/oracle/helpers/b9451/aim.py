from typing import Literal, Protocol, TypedDict

from litellm import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.aim.aim import AimGuardrail
from litellm.types.utils import CallTypesLiteral, LLMResponseTypes
from pydantic import JsonValue as Json

from .scripted import Answer, Returned, attempt, outcome_view, post_call_contents, scripted

HEADER_NAMES = ("content-type", "x-aim-call-id", "x-aim-gateway-key-alias", "x-aim-litellm-hook")


class AimCase(TypedDict):
    vendor: Literal["aim"]
    hook: Literal["pre_call", "moderation", "post_call"]
    api_key: str
    inspect_embeddings: bool
    call_type: CallTypesLiteral
    data: dict[str, Json]
    outputs: list[str]
    key_alias: str | None
    answers: list[Answer]


class AimHooks(Protocol):
    async def async_pre_call_hook(
        self, user_api_key_dict: UserAPIKeyAuth, cache: DualCache, data: dict[str, Json], call_type: CallTypesLiteral
    ) -> Returned: ...

    async def async_moderation_hook(
        self, data: dict[str, Json], user_api_key_dict: UserAPIKeyAuth, call_type: CallTypesLiteral
    ) -> Returned: ...

    async def async_post_call_success_hook(
        self, data: dict[str, Json], user_api_key_dict: UserAPIKeyAuth, response: LLMResponseTypes
    ) -> LLMResponseTypes: ...


def returned_data(returned: Returned) -> dict[str, Json]:
    match returned:
        case dict():
            return returned
        case Exception() | str() | None:
            raise TypeError("the hook returned no request data")


async def call_hook(guardrail: AimHooks, case: AimCase) -> dict[str, Json]:
    user = UserAPIKeyAuth(api_key="aim-oracle-key", key_alias=case["key_alias"])
    data = case["data"]
    match case["hook"]:
        case "pre_call":
            returned = await guardrail.async_pre_call_hook(user, DualCache(), data, case["call_type"])
            return {"data": returned_data(returned)}
        case "moderation":
            returned = await guardrail.async_moderation_hook(data, user, case["call_type"])
            # the during-call dispatch drops this return, so what matters is that it hands the input back
            return {"returns_input": returned_data(returned) is data}
        case "post_call":
            return await post_call_contents(guardrail.async_post_call_success_hook, data, user, case["outputs"])


async def run_aim(case: AimCase) -> Json:
    async with scripted(case["answers"]) as server:
        guardrail: AimHooks = AimGuardrail(
            api_key=case["api_key"],
            api_base=server.base_url,
            inspect_embeddings=case["inspect_embeddings"],
            timeout=None,
        )
        outcome = await attempt(call_hook(guardrail, case))
        return outcome_view(server, outcome, HEADER_NAMES, {"authorization": f"Bearer {case['api_key']}"})
