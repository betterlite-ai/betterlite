import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final

from pydantic import JsonValue
from typing_extensions import TypedDict

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
GOOD_KEY = "sk-test"
BAD_KEY = "bad-key"
CREATED = 1700000000
SPEECH_PATH = "/v1/audio/speech"
SPEECH_BYTES = b"ID3endpoint-speech"
SPEECH_TYPE = "audio/mpeg"
JSON_TYPE = "application/json"
HATE_SCORE: Final = 0.01
CATEGORIES: Final[dict[str, JsonValue]] = {
    "harassment": False,
    "harassment/threatening": False,
    "hate": False,
    "hate/threatening": False,
    "illicit": False,
    "illicit/violent": False,
    "self-harm": False,
    "self-harm/instructions": False,
    "self-harm/intent": False,
    "sexual": False,
    "sexual/minors": False,
    "violence": False,
    "violence/graphic": False,
}
APPLIED: Final[dict[str, JsonValue]] = {
    name: ["text"] for name in CATEGORIES
}
SCORES: Final[dict[str, JsonValue]] = {
    "harassment": HATE_SCORE,
    "harassment/threatening": HATE_SCORE,
    "hate": HATE_SCORE,
    "hate/threatening": HATE_SCORE,
    "illicit": HATE_SCORE,
    "illicit/violent": HATE_SCORE,
    "self-harm": HATE_SCORE,
    "self-harm/instructions": HATE_SCORE,
    "self-harm/intent": HATE_SCORE,
    "sexual": HATE_SCORE,
    "sexual/minors": HATE_SCORE,
    "violence": HATE_SCORE,
    "violence/graphic": HATE_SCORE,
}

# One fixed answer per route, shaped as the family that calls it reads it.
RESPONSES: Final[dict[str, dict[str, JsonValue]]] = {
    "/v1/embeddings": {
        "object": "list",
        "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}],
        "model": "text-embedding-3-small",
        "usage": {"prompt_tokens": 2, "total_tokens": 2},
    },
    "/v1/moderations": {
        "id": "modr-1",
        "model": "omni-moderation-latest",
        "results": [
            {
                "flagged": False,
                "categories": CATEGORIES,
                "category_scores": SCORES,
                "category_applied_input_types": APPLIED,
            }
        ],
    },
    "/v1/completions": {
        "id": "cmpl-1",
        "object": "text_completion",
        "created": CREATED,
        "model": "gpt-3.5-turbo-instruct",
        "choices": [{"index": 0, "text": "hello", "finish_reason": "stop", "logprobs": None}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3},
    },
    "/v1/images/generations": {
        "created": CREATED,
        "data": [{"url": "https://example.test/generated.png", "revised_prompt": "a cat"}],
    },
    "/v1/images/edits": {
        "created": CREATED,
        "data": [{"url": "https://example.test/edited.png", "revised_prompt": "a cat"}],
    },
    "/v1/images/variations": {
        "created": CREATED,
        "data": [{"url": "https://example.test/variation.png", "revised_prompt": None}],
    },
    "/v1/audio/transcriptions": {"text": "hello"},
    "/v1/rerank": {
        "id": "rerank-1",
        "results": [{"index": 0, "relevance_score": 0.9, "document": {"text": "a doc"}}],
        "meta": {"billed_units": {"search_units": 1}},
    },
}

UNAUTHORIZED_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": "Invalid API key provided: bad-key",
        "type": "invalid_request_error",
        "param": None,
        "code": "invalid_api_key",
    }
}


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: bytes


class EndpointServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]
    counts: dict[str, int]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        self.counts = {}
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def record(self, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})
        current = self.counts.get(path)
        self.counts[path] = 1 if current is None else current + 1

    # contextlib.closing calls close, so the serving thread stops with the test
    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[EndpointServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") == f"Bearer {BAD_KEY}":
            self.reply(UNAUTHORIZED, json.dumps(UNAUTHORIZED_BODY).encode(), JSON_TYPE)
            return
        if self.path == SPEECH_PATH:
            self.reply(OK, SPEECH_BYTES, SPEECH_TYPE)
            return
        payload = RESPONSES.get(self.path)
        if payload is None:
            self.send_error(NOT_FOUND)
            return
        self.reply(OK, json.dumps(payload).encode(), JSON_TYPE)

    def reply(self, status: int, content: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)
