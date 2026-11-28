import datetime as dt
from typing import ClassVar, Final, Literal
from uuid import uuid4
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import ConfigDict, Field, JsonValue, field_validator, model_validator
from typing_extensions import Self, override

from betterlite.types.utils.base import InitValue, ResponseObject
from betterlite.types.utils.pricing import MirroredPricingParams


def _as_utc(value: dt.datetime | None) -> dt.datetime | None:
    if value is None:
        return None
    if value.tzinfo is None:
        return value.replace(tzinfo=dt.timezone.utc)
    return value.astimezone(dt.timezone.utc)


class ModelAccessWindow(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    start: dt.time
    end: dt.time
    timezone: str
    team_ids: tuple[str, ...] = Field(min_length=1)

    @field_validator("start", "end")
    @classmethod
    def _naive_wall_clock(cls, value: dt.time) -> dt.time:
        if value.tzinfo is not None:
            raise ValueError("start and end must be local wall-clock times without a UTC offset")
        return value

    @field_validator("timezone")
    @classmethod
    def _known_iana_timezone(cls, value: str) -> str:
        try:
            ZoneInfo(value)
        except (ZoneInfoNotFoundError, ValueError) as exc:
            raise ValueError(f"unknown IANA timezone '{value}'") from exc
        return value

    @field_validator("team_ids")
    @classmethod
    def _non_empty_team_ids(cls, value: tuple[str, ...]) -> tuple[str, ...]:
        if any(not team_id for team_id in value):
            raise ValueError("team_ids entries must be non-empty")
        return value

    @model_validator(mode="after")
    def _start_differs_from_end(self) -> Self:
        if self.start == self.end:
            raise ValueError("start and end must differ")
        return self


def _model_id(value: JsonValue) -> JsonValue:
    match value:
        case bool() | int():
            return str(value)
        case str() | float() | list() | dict() | None:
            return value


class ModelInfo(MirroredPricingParams):
    id: str | None
    db_model: bool = False
    updated_at: dt.datetime | None = None
    updated_by: str | None = None

    created_at: dt.datetime | None = None
    created_by: str | None = None

    base_model: str | None = None
    tier: Literal["free", "paid"] | None = None

    team_id: str | None = None
    team_public_model_name: str | None = None
    member_auto_router: bool = False

    blocked: bool | None = None
    discoverable: bool | None = None

    access_windows: tuple[ModelAccessWindow, ...] | None = None

    MAX_PTU_COUNT: ClassVar[int] = 1_000_000
    MAX_COST_PER_PTU_PER_HOUR: ClassVar[float] = 1_000_000.0

    ptu_count: int | None = None
    cost_per_ptu_per_hour: float | None = None
    ptu_effective_from: dt.datetime | None = None
    ptu_effective_to: dt.datetime | None = None

    allow_fail_open: bool | None = None
    enable_tag_filtering: bool | None = None
    internal_router_model: bool | None = None

    @override
    def __init__(self, id: str | int | None = None, **params: InitValue) -> None:
        model_id: Final = str(uuid4()) if id is None else _model_id(id)
        ResponseObject.__init__(self, id=model_id, **params)

    @model_validator(mode="after")
    def _validate_ptu_bounds(self) -> Self:
        max_count: Final = self.MAX_PTU_COUNT
        if self.ptu_count is not None and not 0 < self.ptu_count <= max_count:
            raise ValueError(f"ptu_count must be a positive integer no greater than {max_count}")
        max_cost: Final = self.MAX_COST_PER_PTU_PER_HOUR
        if (
            self.cost_per_ptu_per_hour is not None
            and not 0 <= self.cost_per_ptu_per_hour <= max_cost
        ):
            raise ValueError(
                f"cost_per_ptu_per_hour must be a finite number between 0 and {max_cost}"
            )
        start: Final = _as_utc(self.ptu_effective_from)
        end: Final = _as_utc(self.ptu_effective_to)
        if start is not None and end is not None and end <= start:
            raise ValueError("ptu_effective_to must be after ptu_effective_from")
        return self

    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")
