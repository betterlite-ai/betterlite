from typing import Unpack

from pydantic import JsonValue
from typing_extensions import TypedDict

from ._transport import UNAUTHORIZED_STATUS, Request, send
from .exceptions import UnauthorizedError

AUTHENTICATION_FAILED = "Authentication failed. Check your API key."
TEAM_LIST_KEYS: frozenset[str] = frozenset({
    "organization_id",
    "page",
    "page_size",
    "sort_by",
    "sort_order",
    "team_alias",
    "team_id",
    "user_id",
})


class TeamListOptions(TypedDict, closed=True, total=False):
    user_id: str | None
    organization_id: str | None
    team_id: str | None
    team_alias: str | None
    page: int
    page_size: int
    sort_by: str | None
    sort_order: str


def _check_options(options: TeamListOptions) -> None:
    for name in options:
        if name not in TEAM_LIST_KEYS:
            raise TypeError(f"list_v2() got an unexpected keyword argument {name!r}")


class TeamsManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers: dict[str, str] = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    def _get(self, path: str, params: dict[str, str]) -> JsonValue:
        request = Request(
            method="GET",
            url=f"{self._base_url}{path}",
            headers=self._get_headers(),
            params=params,
        )
        response = send(request, self._timeout)
        if response.status_code == UNAUTHORIZED_STATUS:
            raise UnauthorizedError(AUTHENTICATION_FAILED)
        response.raise_for_status()
        return response.json()

    def list(self, user_id: str | None = None, organization_id: str | None = None) -> JsonValue:
        params: dict[str, str] = {}
        if user_id:
            params["user_id"] = user_id
        if organization_id:
            params["organization_id"] = organization_id
        return self._get("/team/list", params)

    def list_v2(self, **options: Unpack[TeamListOptions]) -> JsonValue:
        _check_options(options)
        params: dict[str, str] = {}
        page = options.get("page")
        params["page"] = "1" if page is None else str(page)
        page_size = options.get("page_size")
        params["page_size"] = "10" if page_size is None else str(page_size)
        sort_order = options.get("sort_order")
        params["sort_order"] = "asc" if sort_order is None else sort_order
        filters = {
            "user_id": options.get("user_id"),
            "organization_id": options.get("organization_id"),
            "team_id": options.get("team_id"),
            "team_alias": options.get("team_alias"),
            "sort_by": options.get("sort_by"),
        }
        for name, value in filters.items():
            if value:
                params[name] = value
        return self._get("/v2/team/list", params)

    def get_available(self) -> JsonValue:
        params: dict[str, str] = {}
        return self._get("/team/available", params)
