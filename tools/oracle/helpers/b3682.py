from typing import Literal, TypedDict

from litellm.proxy.guardrails.guardrail_registry import InMemoryGuardrailHandler
from litellm.types.guardrails import Guardrail, LitellmParams
from pydantic import TypeAdapter


class Input(TypedDict):
    function: Literal["stable_guardrail_id"]
    guardrail_name: str
    existing_ids: list[str]


class Output(TypedDict):
    id: str


class AssignedId(TypedDict):
    guardrail_id: str


def guardrail(guardrail_name: str, guardrail_id: str | None) -> Guardrail:
    return Guardrail(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L92-L140
        guardrail_id=guardrail_id,
        guardrail_name=guardrail_name,
        litellm_params=LitellmParams(guardrail="tool_permission", mode="pre_call"),
    )


def run(input_value: Input) -> Output:
    name = input_value["guardrail_name"]
    ids = input_value["existing_ids"]
    handler = InMemoryGuardrailHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_registry.py#L469
    handler.IN_MEMORY_GUARDRAILS = {i: guardrail(i, i) for i in ids}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_registry.py#L470
    assigned = TypeAdapter(AssignedId).validate_python(
        handler.initialize_guardrail(guardrail(name, None))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_registry.py#L496-L506
    )
    return {"id": assigned["guardrail_id"]}
