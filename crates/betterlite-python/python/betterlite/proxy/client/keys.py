import builtins
from collections.abc import Iterable, Mapping
from typing import TypedDict, Unpack

from pydantic import JsonValue

from ._bridge import redact_string
from ._transport import (
    UNAUTHORIZED_STATUS,
    HTTPError,
    Request,
    RequestError,
    Response,
    send,
)
from .exceptions import UnauthorizedError


class KeyUpdateError(Exception):
    pass


class KeyListOptions(TypedDict, total=False):
    page: int | None
    size: int | None
    user_id: str | None
    team_id: str | None
    organization_id: str | None
    key_hash: str | None
    key_alias: str | None
    return_full_object: bool | None
    include_team_keys: bool | None
    return_request: bool


class KeyGenerateOptions(TypedDict, total=False):
    models: list[str] | None
    aliases: dict[str, str] | None
    spend: float | None
    duration: str | None
    key_alias: str | None
    team_id: str | None
    user_id: str | None
    budget_id: str | None
    config: Mapping[str, JsonValue] | None
    return_request: bool


class KeyUpdateOptions(TypedDict, total=False):
    models: list[str] | None
    aliases: dict[str, str] | None
    spend: float | None
    duration: str | None
    key_alias: str | None
    team_id: str | None
    user_id: str | None


def _reject_unknown_options(names: Iterable[str], allowed: frozenset[str], call: str) -> None:
    unknown = [name for name in names if name not in allowed]
    if unknown:
        joined = ", ".join(unknown)
        raise TypeError(f"{call}() got unexpected keyword arguments: {joined}")


def _query_value(value: int | None) -> str | None:
    if value is None:
        return None
    return str(value).lower()


def _json_list(values: list[str] | None) -> list[JsonValue] | None:
    if values is None:
        return None
    converted: list[JsonValue] = []
    converted.extend(values)
    return converted


def _json_map(values: Mapping[str, JsonValue] | None) -> dict[str, JsonValue] | None:
    if values is None:
        return None
    return dict(values)


def _key_reply(response: Response) -> dict[str, JsonValue]:
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
            raise RequestError("the key reply is not a JSON object")


def _key_call(
    request: Request,
    timeout: int,
    *,
    return_request: bool,
) -> dict[str, JsonValue] | Request:
    if return_request:
        return request
    return _key_reply(send(request, timeout))


def _list_params(options: KeyListOptions) -> dict[str, str]:
    filters: tuple[tuple[str, str | None], ...] = (
        ("page", _query_value(options.get("page"))),
        ("size", _query_value(options.get("size"))),
        ("user_id", options.get("user_id")),
        ("team_id", options.get("team_id")),
        ("organization_id", options.get("organization_id")),
        ("key_hash", options.get("key_hash")),
        ("key_alias", options.get("key_alias")),
        ("return_full_object", _query_value(options.get("return_full_object"))),
        ("include_team_keys", _query_value(options.get("include_team_keys"))),
    )
    return {name: value for name, value in filters if value is not None}


def _generate_body(options: KeyGenerateOptions) -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {}
    models = options.get("models")
    if models is not None:
        body["models"] = _json_list(models)
    aliases = options.get("aliases")
    if aliases is not None:
        body["aliases"] = _json_map(aliases)
    spend = options.get("spend")
    if spend is not None:
        body["spend"] = spend
    duration = options.get("duration")
    if duration is not None:
        body["duration"] = duration
    key_alias = options.get("key_alias")
    if key_alias is not None:
        body["key_alias"] = key_alias
    team_id = options.get("team_id")
    if team_id is not None:
        body["team_id"] = team_id
    user_id = options.get("user_id")
    if user_id is not None:
        body["user_id"] = user_id
    budget_id = options.get("budget_id")
    if budget_id is not None:
        body["budget_id"] = budget_id
    config = options.get("config")
    if config is not None:
        body["config"] = _json_map(config)
    return body


class KeysManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    def _request(
        self,
        method: str,
        path: str,
        *,
        params: dict[str, str] | None = None,
        json: JsonValue = None,
    ) -> Request:
        return Request(
            method=method,
            url=f"{self._base_url}{path}",
            headers=self._get_headers(),
            params={} if params is None else params,
            json=json,
        )

    def list(self, **options: Unpack[KeyListOptions]) -> dict[str, JsonValue] | Request:
        _reject_unknown_options(options, KeyListOptions.__optional_keys__, "list")
        request = self._request("GET", "/key/list", params=_list_params(options))
        return_request = options.get("return_request") is True
        return _key_call(request, self._timeout, return_request=return_request)

    def generate(self, **options: Unpack[KeyGenerateOptions]) -> dict[str, JsonValue] | Request:
        _reject_unknown_options(options, KeyGenerateOptions.__optional_keys__, "generate")
        request = self._request("POST", "/key/generate", json=_generate_body(options))
        return_request = options.get("return_request") is True
        return _key_call(request, self._timeout, return_request=return_request)

    def delete(
        self,
        keys: builtins.list[str] | None = None,
        key_aliases: builtins.list[str] | None = None,
        *,
        return_request: bool = False,
    ) -> dict[str, JsonValue] | Request:
        body: dict[str, JsonValue] = {
            "keys": _json_list(keys),
            "key_aliases": _json_list(key_aliases),
        }
        request = self._request("POST", "/key/delete", json=body)
        return _key_call(request, self._timeout, return_request=return_request)

    def update(self, key: str, **options: Unpack[KeyUpdateOptions]) -> dict[str, JsonValue]:
        _reject_unknown_options(options, KeyUpdateOptions.__optional_keys__, "update")
        data: dict[str, JsonValue] = {"key": key}
        fields: tuple[tuple[str, JsonValue], ...] = (
            ("key_alias", options.get("key_alias")),
            ("user_id", options.get("user_id")),
            ("team_id", options.get("team_id")),
            ("models", _json_list(options.get("models"))),
            ("spend", options.get("spend")),
            ("duration", options.get("duration")),
            ("aliases", _json_map(options.get("aliases"))),
        )
        data.update({name: value for name, value in fields if value is not None})
        request = self._request("POST", "/key/update", json=data)
        response_text: str | None = None
        try:
            response = send(request, self._timeout)
            response_text = response.text
            reply = _key_reply(response)
        except (RequestError, UnauthorizedError, ValueError):
            raise KeyUpdateError(f"Error updating key: {response_text}") from None
        else:
            return reply

    def info(self, key: str, *, return_request: bool = False) -> dict[str, JsonValue] | Request:
        request = self._request("GET", f"/key/info?key={key}")
        if return_request:
            return request
        response = send(request, self._timeout)
        try:
            response.raise_for_status()
        except HTTPError as error:
            status = None if error.response is None else error.response.status_code
            if status == UNAUTHORIZED_STATUS:
                raise UnauthorizedError(error) from None
            message = redact_string(str(error))
            raise HTTPError(message, response=error.response) from None
        match response.json():
            case dict() as reply:
                return reply
            case list() | str() | bool() | int() | float() | None:
                raise RequestError("the key reply is not a JSON object")
