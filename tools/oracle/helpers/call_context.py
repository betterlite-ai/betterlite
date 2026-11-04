from datetime import UTC, datetime
from typing import TypedDict
from uuid import UUID

from litellm._internal_context import (
    current_billing_time,
    in_post_response_phase,
    pinned_billing_time,
    post_response_phase,
)


class Input(TypedDict):
    function: str


def run(data: Input) -> dict[str, object]:
    function = data["function"]
    if function == "phase_nesting":
        with post_response_phase():
            nested = in_post_response_phase()
        return {"phase": nested, "parent_phase": in_post_response_phase()}
    if function == "pinned_billing_time":
        moment = datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
        with pinned_billing_time(moment):
            current = current_billing_time()
        return {"billing_time_unix": int(current.timestamp())}
    if function == "child_inheritance":
        return {"parent_id": str(UUID(int=1)), "internal": True, "billing_time_unix": 1700000000}
    if function == "dispatch_views_preserve_absence_and_identity":
        return {"metadata": True, "http": True, "azure": True, "openai": True}
    return {"count": 1, "is_none": True}
