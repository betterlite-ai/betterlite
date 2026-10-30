from datetime import datetime
from typing import TypeAlias, cast

from litellm.repositories.unit_of_work import (
    _spend_reset_data,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    result: dict[str, Json] = {}
    for name, case in cast(dict[str, dict[str, Json]], input_value["cases"]).items():
        raw_reset_at = case["budget_reset_at"]
        budget_reset_at = (
            None if raw_reset_at is None else datetime.fromisoformat(cast(str, raw_reset_at))
        )
        spend_decrement = cast(float, case["spend_decrement"])
        data = _spend_reset_data(budget_reset_at, spend_decrement)
        reset_at = cast("datetime | None", data["budget_reset_at"])
        decrement = cast("float | int", cast(dict[str, Json], data["spend"])["decrement"])
        result[name] = {
            "spend": {"decrement": str(decrement)},
            "budget_reset_at": (
                None if reset_at is None else reset_at.isoformat().replace("+00:00", "Z")
            ),
        }
    return result
