from datetime import date, datetime, tzinfo
from typing import ClassVar, Final, NotRequired, Protocol, Self, TypedDict

import litellm
from litellm.proxy.common_utils import model_deprecation as deprecation
from litellm.router import Router
from litellm.types.proxy.model_deprecation import DEFAULT_DEPRECATION_WARN_DAYS
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Meta = dict[str, JsonValue | date | datetime]
EMPTY_COST: Final[dict[str, JsonValue]] = {}
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
SNAPSHOT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Row(TypedDict, total=False):
    model_name: JsonValue
    model_info: JsonValue
    litellm_params: JsonValue


class StoredRow(TypedDict, total=False):
    model_name: JsonValue
    model_info: Meta
    litellm_params: Meta


class Input(TypedDict):
    now: str
    model_cost: dict[str, JsonValue]
    deployments: list[Row] | None
    warn_within_days: NotRequired[int | None]
    today: NotRequired[str | None]


class RouterStorage(Protocol):
    model_list: list[StoredRow]


class FrozenDatetime(datetime):
    instant: ClassVar["FrozenDatetime"]

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(cls.instant.timestamp(), tz)


def marker_date(kind: str, value: str) -> date | None:
    if kind == "date":
        return date.fromisoformat(value.strip())
    if kind == "datetime":
        return FrozenDatetime.fromisoformat(value).date()
    return None


def tagged_value(raw: JsonValue) -> JsonValue | date | datetime:
    match raw:
        case {"kind": str() as kind, "value": str() as marker}:
            return marker_date(kind, marker)
        case None | bool() | int() | float() | str() | list() | dict():
            return raw


def replaced_meta(raw: JsonValue) -> Meta:
    match raw:
        case dict():
            meta: Meta = {}
            for key, value in OBJECT.validate_python(raw).items():
                meta[key] = tagged_value(value)
            return meta
        case None | bool() | int() | float() | str() | list():
            return {}


def stored_row(row: Row) -> StoredRow:
    stored: StoredRow = {}
    model_name = row.get("model_name")
    if model_name is not None:
        stored["model_name"] = model_name
    model_info = row.get("model_info")
    if model_info is not None:
        stored["model_info"] = replaced_meta(model_info)
    params = row.get("litellm_params")
    if params is not None:
        stored["litellm_params"] = replaced_meta(params)
    return stored


def cost_entries(raw: dict[str, JsonValue]) -> dict[str, Meta]:
    entries: dict[str, Meta] = {}
    for key, value in raw.items():
        entries[key] = replaced_meta(value)
    return entries


def explicit_today(raw: str | None) -> date | None:
    if raw is None:
        return None
    return date.fromisoformat(raw)


def warn_days(raw: int | None) -> int:
    if raw is None:
        return DEFAULT_DEPRECATION_WARN_DAYS
    return raw


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    rows = input_value["deployments"]
    if rows is None:
        llm_router: Router | None = None
    else:
        router = Router()
        storage: RouterStorage = router
        storage.model_list = [stored_row(row) for row in rows]
        llm_router = router
    FrozenDatetime.instant = FrozenDatetime.fromisoformat(input_value["now"])
    object.__setattr__(deprecation, "datetime", FrozenDatetime)
    litellm.model_cost = cost_entries(input_value["model_cost"])
    try:
        snapshot = deprecation.collect_model_deprecations(
            llm_router,
            warn_within_days=warn_days(input_value.get("warn_within_days")),
            today=explicit_today(input_value.get("today")),
        )
        message = deprecation.format_deprecation_alert_message(snapshot)
        return {
            "snapshot": SNAPSHOT.validate_json(snapshot.model_dump_json()),
            "message": message,
        }
    finally:
        object.__setattr__(deprecation, "datetime", datetime)
        litellm.model_cost = EMPTY_COST
