from litellm.constants import (
    BEDROCK_REALTIME_PENDING_SESSION_UPDATE_SCOPE_KEY as PENDING_KEY,
    BEDROCK_REALTIME_SESSION_COMMITTED_SCOPE_KEY as COMMITTED_KEY,
)
from litellm.types.realtime import RealtimeResponseTransformInput
from pydantic import JsonValue

from .models import PENDING_ADAPTER, ScopeView


def empty_state() -> RealtimeResponseTransformInput:
    return {
        "session_configuration_request": None,
        "current_output_item_id": None,
        "current_response_id": None,
        "current_delta_chunks": None,
        "current_item_chunks": None,
        "current_conversation_id": None,
        "current_delta_type": None,
    }


def scope_view(scope: dict[str, JsonValue]) -> ScopeView:
    return {
        "pending_session_update": PENDING_ADAPTER.validate_python(scope.get(PENDING_KEY)),
        "session_committed": scope.get(COMMITTED_KEY) is True,
    }
