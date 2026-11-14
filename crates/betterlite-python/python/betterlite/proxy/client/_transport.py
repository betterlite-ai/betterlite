import json
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterator, Mapping
from dataclasses import dataclass, field
from http.client import HTTPResponse

from pydantic import JsonValue
from typing_extensions import override

CLIENT_ERRORS = range(400, 500)
SERVER_ERRORS = range(500, 600)
JSON_CONTENT_TYPE = "application/json"
UNAUTHORIZED_STATUS = 401
NOT_FOUND_STATUS = 404


class RequestError(Exception):
    pass


class HTTPError(RequestError):
    response: "Response | None"

    def __init__(self, message: str, *, response: "Response | None" = None) -> None:
        super().__init__(message)
        self.response = response


@dataclass
class Response:
    status_code: int | None = None
    text: str = ""
    headers: dict[str, str] = field(default_factory=dict)
    reason: str = ""
    url: str = ""
    stream: HTTPResponse | None = None

    def json(self) -> JsonValue:
        parsed: JsonValue = json.loads(self.text)
        return parsed

    def iter_lines(self) -> Iterator[bytes]:
        if self.stream is None:
            for text_line in self.text.splitlines():
                yield text_line.encode("utf-8")
            return
        for raw_line in self.stream:
            yield raw_line.rstrip(b"\r\n")

    def consume(self) -> None:
        if self.stream is None:
            return
        body = self.stream.read()
        self.text = body.decode("utf-8", errors="replace")
        self.stream = None

    def raise_for_status(self) -> None:
        status = self.status_code
        if status is None:
            return
        if status in CLIENT_ERRORS:
            message = f"{status} Client Error: {self.reason} for url: {self.url}"
            raise HTTPError(message, response=self)
        if status in SERVER_ERRORS:
            message = f"{status} Server Error: {self.reason} for url: {self.url}"
            raise HTTPError(message, response=self)


@dataclass(frozen=True)
class Request:
    method: str
    url: str
    headers: dict[str, str] = field(default_factory=dict)
    params: dict[str, str] = field(default_factory=dict)
    json: JsonValue = None
    data: bytes | None = None


class _BodyRequest(urllib.request.Request):
    raw_body: bool = False

    @override
    def has_header(self, header_name: str) -> bool:
        if self.raw_body and header_name == "Content-type":
            return True
        return super().has_header(header_name)


def send(request: Request, timeout: float) -> Response:
    response = stream(request, timeout)
    response.consume()
    return response


def stream(request: Request, timeout: float) -> Response:
    outgoing = _outgoing(request)
    opener = urllib.request.build_opener()
    try:
        raw: HTTPResponse = opener.open(outgoing, timeout=timeout)
    except urllib.error.HTTPError as error:
        body = error.read()
        return Response(
            status_code=error.code,
            text=body.decode("utf-8", errors="replace"),
            headers=dict(error.headers.items()),
            reason=error.reason,
            url=error.url,
        )
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise RequestError(str(error)) from error
    return Response(
        status_code=raw.status,
        headers=dict(raw.headers.items()),
        reason=raw.reason,
        url=raw.url,
        stream=raw,
    )


def _outgoing(request: Request) -> urllib.request.Request:
    url = request.url
    if request.params:
        url = f"{url}?{urllib.parse.urlencode(request.params)}"
    headers = dict(request.headers)
    body = request.data
    if body is None and request.json is not None:
        body = json.dumps(request.json).encode("utf-8")
        if not _has_content_type(headers):
            headers["Content-Type"] = JSON_CONTENT_TYPE
    outgoing = _BodyRequest(url, data=body, headers=headers, method=request.method)
    outgoing.raw_body = body is not None and not _has_content_type(headers)
    return outgoing


def _has_content_type(headers: Mapping[str, str]) -> bool:
    names = (name.lower() for name in headers)
    return "content-type" in names
