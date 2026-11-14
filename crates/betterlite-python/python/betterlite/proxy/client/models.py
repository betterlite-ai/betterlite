import builtins
from collections.abc import Mapping, Sequence
from typing import Literal, overload

from pydantic import JsonValue

from . import _transport
from ._transport import (
    NOT_FOUND_STATUS,
    UNAUTHORIZED_STATUS,
    HTTPError,
    Request,
    RequestError,
    Response,
)
from .exceptions import NotFoundError, UnauthorizedError


def _data(reply: JsonValue) -> list[JsonValue]:
    match reply:
        case {"data": list() as models}:
            return models
        case dict() | list() | str() | bool() | int() | float() | None:
            raise RequestError("the models reply has no data list")


def _model_body(
    identifier: str,
    model_params: Mapping[str, JsonValue],
    *,
    key: str,
    info: Mapping[str, JsonValue] | None,
) -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {key: identifier, "litellm_params": dict(model_params)}
    if info:
        body["model_info"] = dict(info)
    return body


def _find_model(
    models: Sequence[JsonValue], model_id: str | None, model_name: str | None
) -> dict[str, JsonValue] | None:
    for model in models:
        match model:
            case dict() as entry:
                match entry:
                    case {"model_info": {"id": str() as identifier}} if (
                        model_id and identifier == model_id
                    ):
                        return entry
                    case {"model_name": str() as name} if model_name and name == model_name:
                        return entry
                    case dict():
                        continue
            case list() | str() | bool() | int() | float() | None:
                continue
    return None


class ModelsManagementClient:
    def __init__(self, base_url: str, api_key: str | None = None, timeout: int = 30) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout = timeout

    def _get_headers(self) -> dict[str, str]:
        headers: dict[str, str] = {}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    def _request(
        self, method: str, path: str, body: dict[str, JsonValue] | None = None
    ) -> Request:
        return Request(
            method=method,
            url=f"{self._base_url}{path}",
            headers=self._get_headers(),
            json=body,
        )

    def _execute(
        self,
        request: Request,
        *,
        return_request: bool,
        not_found_is_missing: bool = False,
    ) -> JsonValue | Request:
        if return_request:
            return request
        return self._reply(request, not_found_is_missing=not_found_is_missing)

    def _reply(self, request: Request, *, not_found_is_missing: bool = False) -> JsonValue:
        response = _transport.send(request, self._timeout)
        try:
            response.raise_for_status()
        except HTTPError as error:
            status = None if error.response is None else error.response.status_code
            if status == UNAUTHORIZED_STATUS:
                raise UnauthorizedError(error) from error
            if not_found_is_missing and (
                status == NOT_FOUND_STATUS or "not found" in response.text.lower()
            ):
                raise NotFoundError(error) from error
            raise
        return response.json()

    @overload
    def list(self, *, return_request: Literal[True]) -> Request: ...

    @overload
    def list(self, *, return_request: Literal[False] = False) -> builtins.list[JsonValue]: ...

    def list(self, *, return_request: bool = False) -> builtins.list[JsonValue] | Request:
        request = self._request("GET", "/models")
        if return_request:
            return request
        return _data(self._reply(request))

    @overload
    def info(self, *, return_request: Literal[True]) -> Request: ...

    @overload
    def info(self, *, return_request: Literal[False] = False) -> builtins.list[JsonValue]: ...

    def info(self, *, return_request: bool = False) -> builtins.list[JsonValue] | Request:
        request = self._request("GET", "/v1/model/info")
        if return_request:
            return request
        return _data(self._reply(request))

    @overload
    def new(
        self, model_name: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = ..., return_request: Literal[True],
    ) -> Request: ...

    @overload
    def new(
        self, model_name: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = ..., return_request: Literal[False] = False,
    ) -> JsonValue: ...

    def new(
        self, model_name: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = None, return_request: bool = False,
    ) -> JsonValue | Request:
        body = _model_body(model_name, model_params, key="model_name", info=model_info)
        request = self._request("POST", "/model/new", body)
        return self._execute(request, return_request=return_request)

    @overload
    def update(
        self, model_id: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = ..., return_request: Literal[True],
    ) -> Request: ...

    @overload
    def update(
        self, model_id: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = ..., return_request: Literal[False] = False,
    ) -> JsonValue: ...

    def update(
        self, model_id: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = None, return_request: bool = False,
    ) -> JsonValue | Request:
        body = _model_body(model_id, model_params, key="id", info=model_info)
        request = self._request("POST", "/model/update", body)
        return self._execute(request, return_request=return_request, not_found_is_missing=True)

    @overload
    def delete(self, model_id: str, *, return_request: Literal[True]) -> Request: ...

    @overload
    def delete(self, model_id: str, *, return_request: Literal[False] = False) -> JsonValue: ...

    def delete(self, model_id: str, *, return_request: bool = False) -> JsonValue | Request:
        body: dict[str, JsonValue] = {"id": model_id}
        request = self._request("POST", "/model/delete", body)
        return self._execute(request, return_request=return_request, not_found_is_missing=True)

    @overload
    def get(
        self, model_id: str | None = None, model_name: str | None = None, *,
        return_request: Literal[True],
    ) -> Request: ...

    @overload
    def get(
        self, model_id: str | None = None, model_name: str | None = None, *,
        return_request: Literal[False] = False,
    ) -> dict[str, JsonValue]: ...

    def get(
        self, model_id: str | None = None, model_name: str | None = None, *,
        return_request: bool = False,
    ) -> dict[str, JsonValue] | Request:
        if (model_id is None) == (model_name is None):
            raise ValueError("Exactly one of model_id or model_name must be provided")
        if return_request:
            return self.info(return_request=True)
        found = _find_model(self.info(), model_id, model_name)
        if found is not None:
            return found
        if model_id:
            message = f"Model with id={model_id} not found"
        elif model_name:
            message = f"Model with model_name={model_name} not found"
        else:
            message = "Unknown error trying to find model"
        raise NotFoundError(HTTPError(message, response=Response()))
