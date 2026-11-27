import json
import threading
from functools import partial
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final

from pydantic import JsonValue, TypeAdapter
from typing_extensions import Self, override

JSON_TYPE: Final = "application/json"
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class RecordingServer(ThreadingHTTPServer):
    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), RecordingHandler)
        self.replies: dict[str, JsonValue] = {}
        self.statuses: dict[str, int] = {}
        self.locations: dict[str, str] = {}
        self.requests: list[dict[str, JsonValue]] = []
        RecordingHandler.owner = self
        serve = partial(self.serve_forever, poll_interval=0.01)
        self.thread: threading.Thread = threading.Thread(target=serve, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    @override
    def __enter__(self) -> Self:
        return self

    @override
    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class RecordingHandler(BaseHTTPRequestHandler):
    owner: ClassVar[RecordingServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        parsed: JsonValue = None if not body else _JSON.validate_json(body)
        recorded: dict[str, JsonValue] = {
            "path": self.path, "content_type": self.headers.get("Content-Type"),
            "agent": self.headers.get("User-Agent"), "cookie": self.headers.get("Cookie"),
            "body": parsed,
        }
        self.owner.requests.append(recorded)
        location = self.owner.locations.get(self.path)
        status = self.owner.statuses.get(self.path)
        if status is None:
            status = 200 if location is None else 307
        payload = json.dumps(self.owner.replies[self.path]).encode() if location is None else b""
        self.send_response(status)
        if location is not None:
            self.send_header("Location", location)
        self.send_header("content-type", JSON_TYPE)
        self.send_header("content-length", str(len(payload)))
        self.send_header("set-cookie", "session=b166")
        self.end_headers()
        self.wfile.write(payload)
