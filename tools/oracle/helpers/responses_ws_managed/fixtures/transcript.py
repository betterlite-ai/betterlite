import json
import re
from copy import copy
from time import monotonic
from typing import TypedDict

from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from responses_ws_managed import Input
from responses_ws_managed import run as managed_run


class Arguments(TypedDict):
    managed: list[Input]


class Observation(TypedDict):
    frames: list[JsonValue]
    provider_calls: list[JsonValue]
    provider_requests: list[JsonValue]
    quota_calls: list[JsonValue]
    history: dict[str, JsonValue]
    trace: list[JsonValue]
    diagnostics: list[JsonValue]


class WarmupResponse(BaseModel):
    model_config = ConfigDict(extra="allow", strict=True)
    id: str = Field(pattern=r"^resp_warmup_[0-9a-f]{32}$")
    created_at: int = Field(ge=0)


class Warmup(BaseModel):
    model_config = ConfigDict(extra="allow", strict=True)
    response: WarmupResponse


def run(arguments: Arguments) -> dict[str, JsonValue]:
    if not arguments["managed"]:
        raise ValueError("managed sessions are required")
    started = monotonic()
    seconds = min(session["deadline_millis"] for session in arguments["managed"]) / 1000
    frames: list[JsonValue] = []
    sessions: list[JsonValue] = []
    for session in arguments["managed"]:
        run_input = copy(session)
        run_input["deadline_millis"] = int((seconds - (monotonic() - started)) * 1000)
        if run_input["deadline_millis"] <= 0:
            raise ValueError("managed transcript deadline expired")
        answer = TypeAdapter(Observation).validate_python(managed_run(run_input))
        previous: str | None = None
        for index, value in enumerate(answer["frames"]):
            wire_value = value
            if session["normalize_warmup"]:
                warmup = Warmup.model_validate(value)
                identifier = warmup.response.id
                if re.fullmatch(r"resp_warmup_[0-9a-f]{32}", identifier) is None:
                    raise ValueError("invalid generated warmup id")
                if index % 2 == 1 and identifier != previous:
                    raise ValueError("warmup acknowledgements changed id")
                previous = identifier
                warmup.response.id = "resp_warmup_0123456789abcdef0123456789abcdef"
                warmup.response.created_at = 0
                wire_value = TypeAdapter(JsonValue).validate_json(warmup.model_dump_json())
            frames.append(
                {
                    "direction": "provider_to_client",
                    "action": "expect",
                    "text": json.dumps(
                        wire_value, separators=(",", ":"), sort_keys=True, ensure_ascii=False
                    ),
                }
            )
        sessions.append(
            {
                "provider_calls": answer["provider_calls"],
                "provider_requests": answer["provider_requests"],
                "quota_calls": answer["quota_calls"],
                "history": answer["history"],
                "trace": answer["trace"],
                "diagnostics": answer["diagnostics"],
            }
        )
    return {
        "handshake": {"status": 101, "negotiated_subprotocol": None},
        "frames": frames,
        "close": None,
        "managed": sessions,
    }
