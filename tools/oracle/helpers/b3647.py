import json
import os
import threading
from datetime import UTC, datetime
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, TypedDict
from urllib.parse import urlsplit

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    assembly_passthrough_logging_handler as assembly_handler,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

HANDLER = assembly_handler.AssemblyAIPassthroughLoggingHandler
KEY_ENV: Final = "ASSEMBLYAI_API_KEY"
START: Final = datetime(2026, 1, 1, tzinfo=UTC)
END: Final = datetime(2026, 1, 1, 0, 0, 1, tzinfo=UTC)


class Reply(TypedDict):
    status: int
    body: JsonValue


class Call(TypedDict):
    method: str
    url_route: str
    response_body: dict[str, JsonValue]
    transcripts: list[Reply]
    catalog: dict[str, dict[str, JsonValue]]
    api_key: str | None
    polling_interval_seconds: int
    max_polling_attempts: int


class Input(TypedDict):
    calls: list[Call]


class Details(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    response_cost: float | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: Details


def decimal_text(value: float | None) -> str | None:
    if value is None:
        return None
    return format(Decimal(repr(value)).normalize(), "f")


def configure_key(key: str | None) -> None:
    if key is None:
        os.environ.pop(KEY_ENV, None)
        return
    os.environ[KEY_ENV] = key


def make_logging() -> Logging:
    return Logging(
        model="",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id="b3647",
        function_id="b3647",
    )


class Plan:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = replies
        self.requests: list[JsonValue] = []

    def take(self) -> Reply:
        if len(self.replies) > 1:
            return self.replies.pop(0)
        return self.replies[0]


class RecordingServer(ThreadingHTTPServer):
    def __init__(self, plan: Plan) -> None:
        super().__init__(("127.0.0.1", 0), RecordingHandler)
        self.plan = plan
        RecordingHandler.recording = self

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"


class RecordingHandler(BaseHTTPRequestHandler):
    recording: ClassVar[RecordingServer | None] = None

    def do_GET(self) -> None:
        server = RecordingHandler.recording
        if server is None:
            raise RuntimeError("the handler is not on a recording server")
        reply = server.plan.take()
        target = urlsplit(self.path)
        pieces = target.path.split("/", 2)
        record: dict[str, JsonValue] = {
            "base": pieces[1],
            "path": f"/{pieces[2]}",
            "authorization": self.headers.get("Authorization"),
            "content_type": self.headers.get("Content-Type"),
        }
        server.plan.requests.append(record)
        payload = json.dumps(reply["body"]).encode("utf-8")
        self.send_response(reply["status"])
        self.send_header("content-length", str(len(payload)))
        self.send_header("content-type", "application/json")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(payload)


def unlogged_answer(region: str | None, routing_base: str) -> JsonValue:
    return {
        "logged": False,
        "attempts": 0,
        "requests": [],
        "model": None,
        "custom_llm_provider": None,
        "response_cost": None,
        "region": region,
        "routing_base": routing_base,
    }


def answer_for(call: Call, server: RecordingServer) -> JsonValue:
    litellm.model_cost = dict(call["catalog"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    configure_key(call["api_key"])
    url_route = call["url_route"]
    region = HANDLER._get_assembly_region_from_url(url=url_route)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L285
    routing_base = HANDLER._get_assembly_base_url_from_region(region=region)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L296
    if HANDLER._should_log_request(call["method"]) is not True:  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L282
        return unlogged_answer(region, routing_base)
    handler = HANDLER()  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L37
    handler.assembly_ai_base_url = f"{server.base}/us"  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L39
    handler.assembly_ai_eu_base_url = f"{server.base}/eu"  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L40
    handler.polling_interval = call["polling_interval_seconds"]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L46
    handler.max_polling_attempts = call["max_polling_attempts"]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L51
    logging_obj = make_logging()
    handler._handle_assemblyai_passthrough_logging(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/assembly_passthrough_logging_handler.py#L84
        httpx.Response(200),
        call["response_body"],
        logging_obj,
        url_route,
        "success",
        START,
        END,
        cache_hit=False,
    )
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    return {
        "logged": True,
        "attempts": len(server.plan.requests),
        "requests": server.plan.requests,
        "model": details.model,
        "custom_llm_provider": details.custom_llm_provider,
        "response_cost": decimal_text(details.response_cost),
        "region": region,
        "routing_base": routing_base,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = TypeAdapter(Input).validate_python(input_value)
    answers: list[JsonValue] = []
    for call in parsed["calls"]:
        server = RecordingServer(Plan(call["transcripts"]))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        answers.append(answer_for(call, server))
    return {"calls": answers}
