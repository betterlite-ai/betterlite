import asyncio
import logging
import threading
from datetime import datetime
from functools import partial
from http.server import ThreadingHTTPServer
from typing import Final

import litellm
from litellm.exceptions import RateLimitError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.responses.streaming_iterator import (
    ManagedResponsesWebSocketHandler as Handler,
)
from litellm.responses.streaming_iterator import (
    ProjectQuotaCallback,
)
from pydantic import JsonValue
from typing_extensions import override

from .fakes import (
    JSON,
    Answering,
    Input,
    QuotaRow,
    Recorder,
    ScriptedError,
    ScriptedResponses,
    Setup,
)

LOG_NAME: Final = "LiteLLM"
DIAGNOSTIC_PREFIX: Final = "ManagedResponsesWS: "


class Socket:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    async def send_text(self, data: str) -> None:
        self.recorder.sends += 1
        attempt = self.recorder.sends
        if attempt == self.recorder.fails_at:
            raise ScriptedError(self.recorder.fail_message)
        self.recorder.frames.append(data)

    async def receive_text(self) -> str:
        self.recorder.trace.append("receive")
        if self.recorder.incoming_index < len(self.recorder.incoming):
            frame = self.recorder.incoming[self.recorder.incoming_index]
            self.recorder.incoming_index += 1
            return frame
        raise RuntimeError(self.recorder.disconnect)


class ScriptedQuota(ProjectQuotaCallback):
    def __init__(self, recorder: Recorder, row: QuotaRow) -> None:
        self.recorder = recorder
        self.row = row

    @override
    async def enforce_project_io_token_quota_for_frame(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth | None,
        requested_model: str | None,
        estimated_input_tokens: int,
        estimated_output_tokens: int,
    ) -> None:
        del user_api_key_dict
        self.recorder.trace.append("quota")
        self.recorder.quotas.append(
            {
                "model": requested_model,
                "input_tokens": estimated_input_tokens,
                "output_tokens": estimated_output_tokens,
            }
        )
        kind = self.row["kind"]
        if kind == "rate_limit":
            raise RateLimitError(self.row["message"], llm_provider="", model="")
        if kind == "internal":
            raise ScriptedError(self.row["message"])


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(DIAGNOSTIC_PREFIX):
            self.lines.append(message)


class Probe(Handler):
    def history_view(self) -> dict[str, JsonValue]:
        return {
            response_id: JSON.validate_python(messages)
            for response_id, messages in self._session_history.items()
        }


def build_handler(
    setup: Setup, socket: Socket, recorder: Recorder, quota: QuotaRow | None
) -> Probe:
    log_obj = Logging(
        model=setup["model"],
        messages=None,
        stream=True,
        call_type="responses",
        start_time=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    return Probe(
        websocket=socket,
        model=setup["model"],
        logging_obj=log_obj,
        litellm_metadata=setup["litellm_metadata"],
        api_key=setup["api_key"],
        api_base=setup["api_base"],
        timeout=setup["timeout"],
        custom_llm_provider=setup["custom_llm_provider"],
        first_message=setup["first_message"],
        user_api_key_dict=None,
        quota_callbacks=None if quota is None else [ScriptedQuota(recorder, quota)],
        **setup["kwargs"],
    )


def run(input_value: Input) -> dict[str, JsonValue]:
    recorder = Recorder(input_value)
    socket = Socket(recorder)
    server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Answering, recorder))
    base_url = f"http://127.0.0.1:{server.server_address[1]}/v1"
    thread = threading.Thread(target=partial(server.serve_forever, poll_interval=0.01), daemon=True)
    thread.start()
    provider = ScriptedResponses(recorder, base_url)
    handler = build_handler(input_value["setup"], socket, recorder, input_value["quota"])
    capture = Capture()
    logger = logging.getLogger(LOG_NAME)
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(capture)
    logger.setLevel(logging.DEBUG)
    previous_aresponses = litellm.aresponses
    object.__setattr__(litellm, "aresponses", provider)
    try:
        asyncio.run(handler.run())
    finally:
        object.__setattr__(litellm, "aresponses", previous_aresponses)
        server.shutdown()
        server.server_close()
        logger.removeHandler(capture)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    return {
        "frames": [JSON.validate_json(text) for text in recorder.frames],
        "provider_calls": recorder.calls,
        "provider_requests": recorder.requests,
        "quota_calls": recorder.quotas,
        "history": handler.history_view(),
        "trace": recorder.trace,
        "diagnostics": capture.lines,
    }
