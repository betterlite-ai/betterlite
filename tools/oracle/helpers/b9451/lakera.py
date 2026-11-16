from typing import Literal, Protocol, TypedDict

import litellm
from litellm import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.lakera_ai import lakeraAI_Moderation
from litellm.types.guardrails import GuardrailEventHooks, GuardrailItem, Role
from pydantic import JsonValue as Json

from .scripted import Answer, PreCallHooks, attempt, outcome_view, scripted


class LakeraCase(TypedDict):
    vendor: Literal["lakera"]
    hook: Literal["pre_call", "moderation"]
    gate: bool | None
    api_key: str
    moderation_check: Literal["pre_call", "in_parallel"]
    enabled_roles: list[Literal["system", "user", "assistant"]] | None
    add_function_to_prompt: bool
    data: dict[str, Json]
    answers: list[Answer]


class LakeraHooks(PreCallHooks, Protocol):
    async def async_moderation_hook(
        self, data: dict[str, Json], user: UserAPIKeyAuth, call_type: Literal["completion"], /
    ) -> None: ...


def event_hook(case: LakeraCase) -> GuardrailEventHooks | None:
    own, other = GuardrailEventHooks.pre_call, GuardrailEventHooks.during_call
    if case["hook"] == "moderation":
        own, other = other, own
    if case["gate"] is None:
        return None
    return own if case["gate"] else other


async def call_hook(guardrail: LakeraHooks, case: LakeraCase) -> dict[str, Json]:
    user = UserAPIKeyAuth(api_key="lakera-oracle-key")
    if case["hook"] == "pre_call":
        await guardrail.async_pre_call_hook(user, DualCache(), case["data"], "completion")
    else:
        await guardrail.async_moderation_hook(case["data"], user, "completion")
    return {"completed": True}


async def run_lakera(case: LakeraCase) -> Json:
    litellm.add_function_to_prompt = case["add_function_to_prompt"]
    litellm.guardrail_name_config_map.clear()
    if case["enabled_roles"] is not None:
        litellm.guardrail_name_config_map["prompt_injection"] = GuardrailItem(
            callbacks=["lakera_prompt_injection"],
            guardrail_name="prompt_injection",
            enabled_roles=[Role(name) for name in case["enabled_roles"]],
        )
    async with scripted(case["answers"]) as server:
        guardrail: LakeraHooks = lakeraAI_Moderation(
            moderation_check=case["moderation_check"],
            api_base=server.base_url,
            api_key=case["api_key"],
            guardrail_name="lakera",
            event_hook=event_hook(case),
            default_on=True,
        )
        outcome = await attempt(call_hook(guardrail, case))
        return outcome_view(server, outcome, ("content-type",), {"authorization": f"Bearer {case['api_key']}"})
