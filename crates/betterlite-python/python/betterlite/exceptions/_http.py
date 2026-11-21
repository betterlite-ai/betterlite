from collections.abc import Iterator, Mapping, MutableMapping

from typing_extensions import override


class Headers(MutableMapping[str, str]):
    _values: dict[str, str]

    def __init__(self, values: Mapping[str, str] | None = None) -> None:
        self._values = {}
        if values is not None:
            for name, value in values.items():
                self._values[name.lower()] = value

    @override
    def __getitem__(self, name: str) -> str:
        return self._values[name.lower()]

    @override
    def __setitem__(self, name: str, value: str) -> None:
        self._values[name.lower()] = value

    @override
    def __delitem__(self, name: str) -> None:
        del self._values[name.lower()]

    @override
    def __iter__(self) -> Iterator[str]:
        return iter(self._values)

    @override
    def __len__(self) -> int:
        return len(self._values)


class Request:
    method: str
    url: str

    def __init__(self, method: str, url: str) -> None:
        self.method = method
        # httpx percent-encodes the space in LiteLLM's vertex URL, and callers compare .url text.
        self.url = url.replace(" ", "%20")


class Response:
    status_code: int
    headers: Headers
    content: bytes
    _request: Request | None

    def __init__(
        self,
        status_code: int,
        request: Request | None = None,
        headers: Mapping[str, str] | None = None,
        content: bytes | None = None,
    ) -> None:
        self.status_code = status_code
        self.headers = Headers(headers)
        self.content = content if content is not None else b""
        if self.content and "content-length" not in self.headers:
            self.headers["content-length"] = str(len(self.content))
        self._request = request

    @property
    def request(self) -> Request:
        if self._request is None:
            raise RuntimeError("The request instance has not been set on this response.")
        return self._request

    @property
    def text(self) -> str:
        return self.content.decode()

    def has_request(self) -> bool:
        try:
            _ = self.request
        except RuntimeError:
            return False
        return True
