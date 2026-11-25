from collections.abc import Mapping
from typing import Final

import httpx2
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite._cost.helpers import COST_HEADER
from betterlite._cost.objects import PrivateCarrier, private_fields
from betterlite._cost.wire import JsonSource
from betterlite.types.decisions import DecisionsResponse, OpenAIDecisionResponse
from betterlite.types.utils.base import Dumpable, ResponseObject
from betterlite.types.utils.response import process_response_headers as _common_headers

_INTERNAL_PREFIX: Final = "x-litellm-"
_PROVIDER_HEADERS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])
_JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def process_response_headers(
    response_headers: httpx2.Headers | Mapping[str, str],
    preserve_litellm_internal_headers: bool = False,  # ast-grep-ignore: no-long-names  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L453-L456
) -> dict[str, JsonValue]:
    raw: dict[str, str] = dict(response_headers)
    # ast-grep-ignore: no-isinstance
    if isinstance(response_headers, dict) and preserve_litellm_internal_headers:
        internal = {
            key: value for key, value in raw.items() if key.startswith(_INTERNAL_PREFIX)
        }
        rest = {
            key: value for key, value in raw.items() if not key.startswith(_INTERNAL_PREFIX)
        }
        return {**_common_headers(rest), **internal}
    return _common_headers(raw)


def set_response_cost_in_hidden_params(  # ast-grep-ignore: no-long-names
    response: PrivateCarrier, cost: float | None
) -> None:
    if cost is None:
        return
    match private_fields(response).get("_hidden_params"):
        case dict() as params:
            additional = _existing_additional(params)
            additional[COST_HEADER] = cost
            params["additional_headers"] = additional
            return
        case bool() | int() | float() | str() | list() | None:
            raise TypeError("_hidden_params must be an object")


def _existing_additional(params: dict[str, JsonValue]) -> dict[str, JsonValue]:
    match params.get("additional_headers"):
        case Mapping() as headers:
            return dict(headers)
        case bool() | int() | float() | str() | list() | None:
            return {}


def set_provider_response_headers_in_hidden_params(  # ast-grep-ignore: no-long-names
    response: PrivateCarrier, headers: httpx2.Headers | Mapping[str, str]
) -> None:
    match private_fields(response).get("_hidden_params"):
        case dict() as params:
            raw = dict(headers)
            additional = process_response_headers(raw)
            additional.update(_existing_additional(params))
            params["headers"] = _JSON_OBJECT.validate_python(raw)
            params["additional_headers"] = additional
            return
        case bool() | int() | float() | str() | list() | None:
            raise TypeError("_hidden_params must be an object")


def get_provider_response_headers_from_hidden_params(  # ast-grep-ignore: no-long-names
    response: JsonSource | None,
) -> dict[str, str] | None:
    if response is None:
        return None
    hidden: JsonValue = None
    match response:
        case ResponseObject():
            hidden = private_fields(response).get("_hidden_params")
        case DecisionsResponse() | OpenAIDecisionResponse():
            hidden = response.hidden_params
        case Dumpable() | Mapping():
            return None
    match hidden:
        case Mapping() as params:
            try:
                return _PROVIDER_HEADERS.validate_python(params.get("headers"))
            except ValidationError:
                return None
        case bool() | int() | float() | str() | list() | None:
            return None
