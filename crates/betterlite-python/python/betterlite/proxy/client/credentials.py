from collections.abc import Mapping

from pydantic import JsonValue

from ._transport import (
    UNAUTHORIZED_STATUS,
    HTTPError,
    Request,
    RequestError,
    Response,
    send,
)
from .exceptions import UnauthorizedError

CredentialReply = dict[str, JsonValue] | Request


def _credential_reply(response: Response) -> dict[str, JsonValue]:
    try:
        response.raise_for_status()
    except HTTPError as error:
        if error.response is not None and error.response.status_code == UNAUTHORIZED_STATUS:
            raise UnauthorizedError(error) from error
        raise
    match response.json():
        case dict() as reply:
            return reply
        case list() | str() | bool() | int() | float() | None:
            raise RequestError("the credential reply is not a JSON object")


class CredentialsManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    def _request(self, method: str, path: str, body: JsonValue = None) -> Request:
        return Request(
            method=method,
            url=f"{self._base_url}{path}",
            headers=self._get_headers(),
            json=body,
        )

    def list(self, *, return_request: bool = False) -> CredentialReply:
        request = self._request("GET", "/credentials")
        if return_request:
            return request
        return _credential_reply(send(request, self._timeout))

    def create(
        self,
        credential_name: str,
        credential_info: Mapping[str, JsonValue],
        credential_values: Mapping[str, JsonValue],
        *,
        return_request: bool = False,
    ) -> dict[str, JsonValue] | Request:
        data: dict[str, JsonValue] = {
            "credential_name": credential_name,
            "credential_info": dict(credential_info),
            "credential_values": dict(credential_values),
        }
        request = self._request("POST", "/credentials", data)
        if return_request:
            return request
        return _credential_reply(send(request, self._timeout))

    def delete(self, credential_name: str, *, return_request: bool = False) -> CredentialReply:
        request = self._request("DELETE", f"/credentials/{credential_name}")
        if return_request:
            return request
        return _credential_reply(send(request, self._timeout))

    def get(self, credential_name: str, *, return_request: bool = False) -> CredentialReply:
        request = self._request("GET", f"/credentials/by_name/{credential_name}")
        if return_request:
            return request
        return _credential_reply(send(request, self._timeout))
