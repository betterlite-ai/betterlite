from typing import Literal, TypedDict

from litellm.proxy.guardrails.guardrail_registry import InMemoryGuardrailHandler
from litellm.types.guardrails import Guardrail, LitellmParams


class Input(TypedDict):
    function: Literal["stable_guardrail_id"]
    guardrail_name: str
    existing_ids: list[str]


class Output(TypedDict):
    id: str


def held(guardrail_id: str) -> Guardrail:
    return Guardrail(
        guardrail_id=guardrail_id,
        guardrail_name=f"held-{guardrail_id}",
        litellm_params=LitellmParams(guardrail="tool_permission", mode="pre_call"),
    )


def run(input_value: Input) -> Output:
    handler = InMemoryGuardrailHandler()
    handler.IN_MEMORY_GUARDRAILS = {
        guardrail_id: held(guardrail_id)
        for guardrail_id in input_value["existing_ids"]
    }
    guardrail = Guardrail(
        guardrail_name=input_value["guardrail_name"],
        litellm_params=LitellmParams(guardrail="tool_permission", mode="pre_call"),
    )
    parsed = handler.initialize_guardrail(guardrail)
    guardrail_id = None if parsed is None else parsed.get("guardrail_id")
    if guardrail_id is None:
        raise ValueError("the guardrail did not get a stable id")
    return {"id": guardrail_id}
