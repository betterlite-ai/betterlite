import urllib.parse
from collections.abc import Mapping

from betterlite.types.utils.values import JsonValue

from ._transport import Request, send

FORM_CONTENT_TYPE = "application/x-www-form-urlencoded"


class HTTPClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def request(
        self,
        method: str,
        uri: str,
        *,
        data: Mapping[str, str] | list[tuple[str, str]] | bytes | None = None,
        json: JsonValue = None,
        headers: dict[str, str] | None = None,
        params: dict[str, str] | None = None,
    ) -> JsonValue:
        url = f"{self._base_url}/{uri.lstrip('/')}"
        request_headers = {} if headers is None else dict(headers)
        if self._api_key:
            request_headers["Authorization"] = f"Bearer {self._api_key}"
        payload = _payload(data, request_headers)
        request = Request(
            method=method,
            url=url,
            headers=request_headers,
            params={} if params is None else params,
            json=json if payload is None else None,
            data=payload,
        )
        response = send(request, self._timeout)
        response.raise_for_status()
        return response.json()


def _payload(
    data: Mapping[str, str] | list[tuple[str, str]] | bytes | None,
    headers: dict[str, str],
) -> bytes | None:
    if data is None:
        return None
    match data:
        case bytes():
            return data
        case list() | Mapping():
            encoded = urllib.parse.urlencode(data).encode("utf-8")
            names = (name.lower() for name in headers)
            if "content-type" not in names:
                headers["Content-Type"] = FORM_CONTENT_TYPE
            return encoded
