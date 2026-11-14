import json
from collections.abc import Iterator, Mapping, Sequence
from typing import Literal, Unpack, overload

from pydantic import JsonValue
from typing_extensions import TypedDict

from . import _transport
from ._transport import UNAUTHORIZED_STATUS, HTTPError, Request, Response
from .exceptions import UnauthorizedError

DATA_PREFIX = "data: "
DONE_MARKER = "[DONE]"
COMPLETION_KEYS = frozenset(
    {
        "temperature",
        "top_p",
        "n",
        "max_tokens",
        "presence_penalty",
        "frequency_penalty",
        "user",
    }
)


class CompletionOptions(TypedDict, closed=True, total=False):
    temperature: float | None
    top_p: float | None
    n: int | None
    max_tokens: int | None
    presence_penalty: float | None
    frequency_penalty: float | None
    user: str | None


def _check_options(method: str, options: CompletionOptions) -> None:
    for name in options:
        if name not in COMPLETION_KEYS:
            raise TypeError(f"{method}() got an unexpected keyword argument {name!r}")


def _json_messages(messages: Sequence[Mapping[str, JsonValue]]) -> list[JsonValue]:
    converted: list[JsonValue] = []
    converted.extend(dict(message) for message in messages)
    return converted


def _add_options(body: dict[str, JsonValue], options: CompletionOptions) -> None:
    temperature = options.get("temperature")
    if temperature is not None:
        body["temperature"] = temperature
    top_p = options.get("top_p")
    if top_p is not None:
        body["top_p"] = top_p
    n = options.get("n")
    if n is not None:
        body["n"] = n
    max_tokens = options.get("max_tokens")
    if max_tokens is not None:
        body["max_tokens"] = max_tokens
    presence_penalty = options.get("presence_penalty")
    if presence_penalty is not None:
        body["presence_penalty"] = presence_penalty
    frequency_penalty = options.get("frequency_penalty")
    if frequency_penalty is not None:
        body["frequency_penalty"] = frequency_penalty
    user = options.get("user")
    if user is not None:
        body["user"] = user


def _completion_body(
    model: str,
    messages: Sequence[Mapping[str, JsonValue]],
    *,
    streaming: bool,
    options: CompletionOptions,
) -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {
        "model": model,
        "messages": _json_messages(messages),
    }
    if streaming:
        body["stream"] = True
    _add_options(body, options)
    return body


def _raise_failure(response: Response) -> None:
    try:
        response.raise_for_status()
    except HTTPError as error:
        status = None if error.response is None else error.response.status_code
        if status == UNAUTHORIZED_STATUS:
            raise UnauthorizedError(error) from error
        raise


class ChatClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 600) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers: dict[str, str] = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    def _completion_request(
        self,
        model: str,
        messages: Sequence[Mapping[str, JsonValue]],
        *,
        streaming: bool,
        options: CompletionOptions,
    ) -> Request:
        body = _completion_body(model, messages, streaming=streaming, options=options)
        return Request(
            method="POST",
            url=f"{self._base_url}/chat/completions",
            headers=self._get_headers(),
            json=body,
        )

    @overload
    def completions(
        self,
        model: str,
        messages: Sequence[Mapping[str, JsonValue]],
        *,
        return_request: Literal[True],
        **options: Unpack[CompletionOptions],
    ) -> Request: ...

    @overload
    def completions(
        self,
        model: str,
        messages: Sequence[Mapping[str, JsonValue]],
        *,
        return_request: Literal[False] = False,
        **options: Unpack[CompletionOptions],
    ) -> JsonValue: ...

    def completions(
        self,
        model: str,
        messages: Sequence[Mapping[str, JsonValue]],
        *,
        return_request: bool = False,
        **options: Unpack[CompletionOptions],
    ) -> JsonValue | Request:
        _check_options("completions", options)
        request = self._completion_request(model, messages, streaming=False, options=options)
        if return_request:
            return request
        response = _transport.send(request, self._timeout)
        _raise_failure(response)
        return response.json()

    def completions_stream(
        self,
        model: str,
        messages: Sequence[Mapping[str, JsonValue]],
        **options: Unpack[CompletionOptions],
    ) -> Iterator[JsonValue]:
        _check_options("completions_stream", options)
        request = self._completion_request(model, messages, streaming=True, options=options)
        response = _transport.stream(request, self._timeout)
        _raise_failure(response)
        for raw_line in response.iter_lines():
            if not raw_line:
                continue
            line = raw_line.decode("utf-8")
            if not line.startswith(DATA_PREFIX):
                continue
            data = line[len(DATA_PREFIX) :]
            if data.strip() == DONE_MARKER:
                break
            try:
                chunk: JsonValue = json.loads(data)
            except json.JSONDecodeError:
                continue
            yield chunk
