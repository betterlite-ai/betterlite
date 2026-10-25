from datetime import datetime
from functools import partial
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.repositories import daily_activity_sql as source
from litellm.repositories.daily_activity_sql import (
    _key_spend_select,  # pyright: ignore[reportPrivateUsage]
    _ptu_flat_cost_select,  # pyright: ignore[reportPrivateUsage]
    _rollup_metric_select,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.repositories.daily_activity import DailyActivityScope
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def evaluate(case: dict[str, Json]) -> Json:
    scope = TypeAdapter(DailyActivityScope).validate_python(case["scope"])
    now = datetime.fromisoformat(str(case["utc_now"]))
    adjusted = partial(source.adjust_dates_for_timezone, utc_now=now)

    with patch.object(source, "adjust_dates_for_timezone", adjusted):
        match case["function"]:
            case "dates":
                dates = adjusted(
                    scope.start_date,
                    scope.end_date,
                    scope.timezone_offset_minutes,
                    scope.include_current_utc_day,
                )
                query = source.SqlQuery("", dates)
            case "where":
                sql, params = source.build_where_clause(
                    scope, start_index=int(str(case["start_index"]))
                )
                query = source.SqlQuery(sql, params)
            case "aggregate":
                query = source.build_aggregated_sql(scope, api_key_limit=int(str(case["limit"])))
            case "entity":
                query = source.build_entity_rollup_sql(scope, api_key_limit=int(str(case["limit"])))
            case "page":
                query = source.build_key_page_sql(
                    scope, limit=int(str(case["limit"])), offset=int(str(case["offset"]))
                )
            case "ptu":
                query = source.SqlQuery(
                    _ptu_flat_cost_select(scope.table, aggregate=bool(case["aggregate"])), ()
                )
            case "metrics":
                query = source.SqlQuery(_rollup_metric_select(scope.table), ())
            case "key_metrics":
                query = source.SqlQuery(_key_spend_select(), ())
            case _:
                raise ValueError("unknown function")
    return {"sql": " ".join(query.sql.split()), "params": cast(Json, list(query.params))}


def run(input_value: dict[str, Json]) -> Json:
    cases = cast(dict[str, dict[str, Json]], input_value["cases"])
    output: dict[str, Json] = {}
    for name, case in cases.items():
        try:
            output[name] = {"query": evaluate(case), "error": None}
        except (ValueError, OverflowError) as error:
            message = (
                "timezone adjustment overflow" if isinstance(error, OverflowError) else str(error)
            )
            output[name] = {"query": None, "error": message}
    return output
