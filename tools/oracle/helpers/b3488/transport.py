from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Final

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue, TypeAdapter

from .case import Case

litellm.suppress_debug_info = True
JSON: Final = TypeAdapter(JsonValue)
DETAILS: Final = TypeAdapter(dict[str, JsonValue])
START: Final = datetime(2026, 1, 1, tzinfo=UTC)
KEPT: Final = frozenset({"content-type", "x-goog-api-key"})


class Recorder:
    def __init__(self, case: Case) -> None:
        self.status = case["reply"]["status"]
        self.headers = case["reply"]["headers"]
        self.body = case["reply"]["body"]
        self.url = ""
        self.method = ""
        self.seen: dict[str, str] = {}
        self.seen_body: JsonValue = None
        names = set(KEPT)
        extra = case["extra_headers"]
        if extra is not None:
            names.update(name.lower() for name in extra)
        self.keep: frozenset[str] = frozenset(names)

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.method = request.method
        kept: dict[str, str] = {}
        for name, value in request.headers.items():
            if name.lower() in self.keep:
                kept[name.lower()] = value
        self.seen = kept
        self.seen_body = JSON.validate_json(request.content)
        response = httpx.Response(
            self.status,
            headers=self.headers,
            content=self.body.encode(),
            request=request,
        )
        response.headers = httpx.Headers(self.headers)
        return response

    def view(self) -> dict[str, JsonValue]:
        return {
            "url": self.url,
            "method": self.method,
            "headers": JSON.validate_python(self.seen),
            "body": self.seen_body,
        }


@dataclass
class Run:
    recorder: Recorder
    logging_obj: Logging


def logging_object(case: Case) -> Logging:
    return Logging(
        model=case["model"],
        messages=None,
        stream=case["stream"],
        call_type="generate_content",
        start_time=START,
        litellm_call_id="call-1",
        function_id="function-1",
        litellm_trace_id="trace-1",
        supports_correlation_logging=False,
    )
