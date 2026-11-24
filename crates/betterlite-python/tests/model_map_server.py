import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib.resources import files
from types import TracebackType
from typing import ClassVar, Final, Self

from typing_extensions import override

HOST: Final = "127.0.0.1"
OK: Final = 200
RETRYABLE: Final = 429
GOOD_PATH: Final = "/good.json"
CORRUPT_PATH: Final = "/corrupt.json"
RETRY_LITELLM_PATH: Final = "/retry-litellm.json"
RETRY_BETTERLITE_PATH: Final = "/retry-betterlite.json"


def bundled_body() -> bytes:
    resource = files("litellm").joinpath("model_prices_and_context_window_backup.json")
    return resource.read_bytes()


class ModelMapServer(ThreadingHTTPServer):
    daemon_threads = True
    visits: set[str]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.visits = set()
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def first_visit(self, path: str) -> bool:
        seen = path in self.visits
        self.visits.add(path)
        return not seen

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
    owner: ClassVar[ModelMapServer]

    def do_GET(self) -> None:
        if self.path == CORRUPT_PATH:
            self.reply(OK, b"{not json", {})
            return
        if self.path in (RETRY_LITELLM_PATH, RETRY_BETTERLITE_PATH) and self.owner.first_visit(
            self.path
        ):
            self.reply(RETRYABLE, b"{}", {"Retry-After": "0"})
            return
        self.reply(OK, bundled_body(), {"ETag": f"etag{self.path}"})

    def reply(self, status: int, body: bytes, headers: dict[str, str]) -> None:
        self.send_response(status)
        for name, value in headers.items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
