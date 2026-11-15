from typing import Literal, TypedDict, TypeVar

from litellm.proxy.guardrails.guardrail_registry import InMemoryGuardrailHandler
from litellm.types.guardrails import Guardrail
from typing_extensions import override

Key = TypeVar("Key")


class Input(TypedDict):
    function: Literal["stable_guardrail_id"]
    guardrail_name: str
    existing_ids: list[str]


class Output(TypedDict):
    id: str


# the id check only asks whether a key is held, so ids with no guardrail behind them are enough
class HeldIds(dict[str, Guardrail]):
    def __init__(self, ids: frozenset[str]) -> None:
        self.ids = ids

    @override
    def __contains__(self, key: Key) -> bool:
        return key in self.ids


class Probe(InMemoryGuardrailHandler):
    def __init__(self, ids: frozenset[str]) -> None:
        self.IN_MEMORY_GUARDRAILS = HeldIds(ids)

    def stable_id(self, guardrail_name: str) -> Output:
        guardrail_id = self._stable_guardrail_id(guardrail_name)
        return {"id": guardrail_id}


def run(input_value: Input) -> Output:
    probe = Probe(frozenset(input_value["existing_ids"]))
    return probe.stable_id(input_value["guardrail_name"])
