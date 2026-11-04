import os
from typing import TypedDict
from unittest.mock import patch

from litellm.litellm_core_utils.env_utils import (
    get_env_int,
    get_env_int_in_range,
    get_env_int_or_none,
)


class Input(TypedDict):
    value: str | None
    default: int
    minimum: int
    maximum: int


def run(data: Input) -> dict[str, int | None]:
    value = data.get("value")
    with patch.dict(os.environ):
        if value is None:
            _ = os.environ.pop("VALUE", None)
        else:
            os.environ["VALUE"] = value
        return {
            "int": get_env_int("VALUE", data["default"]),
            "range": get_env_int_in_range(
                "VALUE", data["default"], data["minimum"], data["maximum"]
            ),
            "optional": get_env_int_or_none("VALUE"),
        }
