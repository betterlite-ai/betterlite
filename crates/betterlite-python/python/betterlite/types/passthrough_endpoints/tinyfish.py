from typing import Final

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict

# ast-grep-ignore: no-long-names
TINYFISH_AGENT_DEFAULT_API_BASE: Final = "https://agent.tinyfish.ai"
TINYFISH_AGENT_DOCS_URL: Final = "https://docs.tinyfish.ai/agent-api"
TINYFISH_DEFAULT_COST_PER_STEP: Final = 0.016
TINYFISH_MODEL_NAME: Final = "tinyfish/automation-run"
# ast-grep-ignore: no-long-names
TINYFISH_POLLING_INTERVAL_SECONDS: Final = 5.0
TINYFISH_MAX_POLLING_SECONDS: Final = 3600.0
# ast-grep-ignore: no-long-names
TINYFISH_MAX_CONSECUTIVE_POLL_FAILURES: Final = 12
TINYFISH_TERMINAL_RUN_STATUSES: Final = frozenset({"COMPLETED", "FAILED", "CANCELLED"})
# ast-grep-ignore: no-long-names
TINYFISH_AUTHENTICATED_RUN_FIELDS: Final = frozenset(
    {"use_profile", "profile_id", "use_vault", "credential_item_ids"}
)
# ast-grep-ignore: no-long-names
TINYFISH_REJECTED_ENVELOPE_FIELDS: Final = frozenset(
    {"custom_body", "stream", "query_params"}
)
# ast-grep-ignore: no-long-names
TINYFISH_PASSTHROUGH_TIMEOUT_SECONDS: Final = 1500.0

_RUN_SUBMIT_PATHS: Final = frozenset(
    {
        ("v1", "automation", "run"),
        ("v1", "automation", "run-async"),
        ("v1", "automation", "run-sse"),
    }
)
_UNSAFE_SEGMENTS: Final = frozenset({"", ".", ".."})
_RUN_READ_SEGMENTS: Final = 3
_RUN_CANCEL_SEGMENTS: Final = 4


class TinyfishRun(TypedDict, total=False):
    run_id: ReadOnly[str | None]
    status: ReadOnly[str | None]
    num_of_steps: ReadOnly[int | None]
    result: ReadOnly[JsonValue]
    error: ReadOnly[JsonValue]
    type: ReadOnly[str | None]


def is_allowed_tinyfish_endpoint(method: str, path: str) -> bool:
    segments: Final = tuple(path.split("/")[1:])
    if not path.startswith("/") or any(segment in _UNSAFE_SEGMENTS for segment in segments):
        return False
    if method == "POST" and segments in _RUN_SUBMIT_PATHS:
        return True
    runs_path: Final = segments[:2] == ("v1", "runs")
    if method == "GET" and runs_path and len(segments) == _RUN_READ_SEGMENTS:
        return True
    return (
        method == "POST" and runs_path
        and len(segments) == _RUN_CANCEL_SEGMENTS
        and segments[3] == "cancel"
    )
