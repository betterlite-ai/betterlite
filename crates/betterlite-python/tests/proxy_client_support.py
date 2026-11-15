import json
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Self

from pydantic import JsonValue
from typing_extensions import override

HOST = "127.0.0.1"


@dataclass(frozen=True)
class Seen:
    method: str
    path: str
    query: str
    authorization: str | None
    content_type: str | None
    body: JsonValue


@dataclass(frozen=True)
class Answer:
    status: int
    text: str
    delay: float


class RecordingServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.requests: list[Seen] = []
        self.answers: list[Answer] = []
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()
        _Handler.recording = self

    def reply(self, status: int, body_text: str, *, delay: float = 0.0) -> None:
        with self.lock:
            self.answers.append(Answer(status, body_text, delay))

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

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


class _Handler(BaseHTTPRequestHandler):
    recording: ClassVar["RecordingServer | None"] = None

    def do_GET(self) -> None:
        self._respond("GET")

    def do_POST(self) -> None:
        self._respond("POST")

    def do_PUT(self) -> None:
        self._respond("PUT")

    def do_DELETE(self) -> None:
        self._respond("DELETE")

    def _respond(self, method: str) -> None:
        server = _Handler.recording
        if server is None:
            raise RuntimeError("the handler is not on a RecordingServer")
        lengths = self.headers.get_all("Content-Length")
        length = 0 if lengths is None else int(lengths[0])
        raw = self.rfile.read(length)
        path, _, query = self.path.partition("?")
        seen = Seen(
            method=method,
            path=path,
            query=query,
            authorization=self.headers.get("Authorization"),
            content_type=self.headers.get("Content-Type"),
            body=_body(raw),
        )
        with server.lock:
            server.requests.append(seen)
            queued = server.answers.pop(0) if server.answers else Answer(500, "", 0.0)
        time.sleep(queued.delay)
        payload = queued.text.encode("utf-8")
        self.send_response(queued.status)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def _body(raw: bytes) -> JsonValue:
    text = raw.decode("utf-8", errors="replace")
    if not text:
        return None
    try:
        parsed: JsonValue = json.loads(text)
    except ValueError:
        return text
    return parsed
