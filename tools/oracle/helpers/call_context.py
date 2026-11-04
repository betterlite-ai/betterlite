from datetime import UTC, datetime
from types import SimpleNamespace
from typing import TypedDict, cast
from uuid import UUID

from litellm._internal_context import (
    current_billing_time,
    in_post_response_phase,
    pinned_billing_time,
    post_response_phase,
)
from litellm.main import (
    _dispatch_client_azure,  # pyright: ignore[reportPrivateUsage]
    _dispatch_client_http,  # pyright: ignore[reportPrivateUsage]
    _dispatch_client_openai,  # pyright: ignore[reportPrivateUsage]
    _dispatch_metadata,  # pyright: ignore[reportPrivateUsage]
    _locals_snapshot,  # pyright: ignore[reportPrivateUsage]
    _provider_config_items,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.completion import (
    _CompletionDispatchContext,  # pyright: ignore[reportPrivateUsage]
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
        metadata = {"marker": object()}
        client = object()
        context = cast(
            _CompletionDispatchContext,
            SimpleNamespace(metadata=metadata, client=client),
        )
        return {
            "metadata": _dispatch_metadata(context) is metadata,
            "http": _dispatch_client_http(context) is client,
            "azure": _dispatch_client_azure(context) is client,
            "openai": _dispatch_client_openai(context) is client,
        }
    values = {"none": None}
    items = list(_provider_config_items(values))
    snapshot = _locals_snapshot(values)
    return {"count": len(items), "is_none": items[0][1] is None and snapshot is values}
