from typing import Literal, TypedDict

from litellm.proxy.guardrails.guardrail_registry import InMemoryGuardrailHandler


class Input(TypedDict):
    function: Literal["stable_guardrail_id"]
    guardrail_name: str
    existing_ids: list[str]


class Output(TypedDict):
    id: str


def run(input_value: Input) -> Output:
    handler = InMemoryGuardrailHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_registry.py#L469
    # the id check only asks whether a key is held, so a key with no guardrail behind it is enough
    handler.IN_MEMORY_GUARDRAILS = {guardrail_id: None for guardrail_id in input_value["existing_ids"]}  # type: ignore[assignment, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_registry.py#L470
    guardrail_id = handler._stable_guardrail_id(input_value["guardrail_name"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_registry.py#L489
    return {"id": guardrail_id}
