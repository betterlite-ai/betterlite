from collections.abc import Mapping

from pydantic import JsonValue

from ._transport import (
    NOT_FOUND_STATUS,
    UNAUTHORIZED_STATUS,
    Request,
    Response,
    send,
)
from .exceptions import NotFoundError, UnauthorizedError


class UsersManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        return headers

    def _send(self, request: Request) -> Response:
        response = send(request, self.timeout)
        if response.status_code == UNAUTHORIZED_STATUS:
            raise UnauthorizedError(response.text)
        return response

    def _post(self, path: str, body: JsonValue) -> JsonValue:
        request = Request(
            method="POST",
            url=f"{self.base_url}{path}",
            headers=self._get_headers(),
            json=body,
        )
        response = self._send(request)
        response.raise_for_status()
        return response.json()

    def list_users(self, params: dict[str, str] | None = None) -> JsonValue:
        request = Request(
            method="GET",
            url=f"{self.base_url}/user/list",
            headers=self._get_headers(),
            params={} if params is None else params,
        )
        response = self._send(request)
        response.raise_for_status()
        reply = response.json()
        match reply:
            case dict() if "users" in reply:
                return reply["users"]
            case dict() | list() | str() | bool() | int() | float() | None:
                return reply

    def get_user(self, user_id: str | None = None) -> JsonValue:
        url = f"{self.base_url}/user/info"
        return self._user_info(url, user_id)

    def get_user_v2(self, user_id: str | None = None) -> JsonValue:
        url = f"{self.base_url}/v2/user/info"
        return self._user_info(url, user_id)

    def _user_info(self, url: str, user_id: str | None) -> JsonValue:
        params: dict[str, str] = {}
        if user_id:
            params["user_id"] = user_id
        request = Request(method="GET", url=url, headers=self._get_headers(), params=params)
        response = self._send(request)
        if response.status_code == NOT_FOUND_STATUS:
            raise NotFoundError(response.text)
        response.raise_for_status()
        return response.json()

    def create_user(self, user_data: Mapping[str, JsonValue]) -> JsonValue:
        body: JsonValue = dict(user_data)
        return self._post("/user/new", body)

    def delete_user(self, user_ids: list[str]) -> JsonValue:
        identifiers: list[JsonValue] = []
        identifiers.extend(user_ids)
        return self._post("/user/delete", {"user_ids": identifiers})
