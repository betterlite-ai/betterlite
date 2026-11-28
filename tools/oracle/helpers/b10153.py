from typing import Literal, TypedDict

from litellm.router_strategy.complexity_router.config import ContextCompactionConfig
from litellm.router_strategy.complexity_router.context_compaction import (
    CompactionState,
    compaction_pending,
)
from pydantic import JsonValue


class Input(TypedDict):
    state: Literal["absent", "unarmed", "armed"]
    payload: dict[str, JsonValue]


def run(input_value: Input) -> dict[str, bool]:
    kwargs: dict[str, JsonValue | CompactionState] = {**input_value["payload"]}
    if input_value["state"] == "unarmed":
        kwargs["_context_compaction_state"] = CompactionState(config=None)
    elif input_value["state"] == "armed":
        kwargs["_context_compaction_state"] = CompactionState(config=ContextCompactionConfig())
    return {"pending": compaction_pending(kwargs)}
