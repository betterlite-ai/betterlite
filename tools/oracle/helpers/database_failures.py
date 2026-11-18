from typing import TypedDict

from litellm.proxy.db.log_db_metrics import _safe_db_event_metadata  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/log_db_metrics.py#L17
from pydantic import JsonValue


class Input(TypedDict):
    values: list[JsonValue]


def run(input_value: Input) -> list[dict[str, str] | None]:
    return [_safe_db_event_metadata({"table_name": value}) for value in input_value["values"]]
