from typing import Literal, overload

from pydantic import JsonValue

from ._transport import UNAUTHORIZED_STATUS, HTTPError, Request, RequestError, send
from .exceptions import UnauthorizedError


class ModelGroupsManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers: dict[str, str] = {}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    @overload
    def info(self, *, return_request: Literal[True]) -> Request: ...

    @overload
    def info(self, *, return_request: Literal[False] = False) -> JsonValue: ...

    def info(self, *, return_request: bool = False) -> JsonValue | Request:
        request = Request(
            method="GET",
            url=f"{self._base_url}/model_group/info",
            headers=self._get_headers(),
        )
        if return_request:
            return request
        response = send(request, self._timeout)
        try:
            response.raise_for_status()
        except HTTPError as error:
            status = None if error.response is None else error.response.status_code
            if status == UNAUTHORIZED_STATUS:
                raise UnauthorizedError(error) from error
            raise
        reply = response.json()
        match reply:
            case dict() if "data" in reply:
                return reply["data"]
            case dict() | list() | str() | bool() | int() | float() | None:
                raise RequestError("the model group info reply has no data")
