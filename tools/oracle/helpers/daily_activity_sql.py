from typing import TypeAlias, cast

from litellm.repositories.daily_activity_sql import build_where_clause
from litellm.types.repositories.daily_activity import DailyActivityScope
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    if input_value.get("function") != "where":
        raise ValueError("unknown function")
    scope = TypeAdapter(DailyActivityScope).validate_python(input_value["scope"])
    sql, params = build_where_clause(scope)
    return {"sql": sql, "params": cast(Json, list(params))}
