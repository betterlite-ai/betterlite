from typing import TypeAlias, cast

from litellm.repositories import daily_activity_sql as source
from litellm.types.repositories.daily_activity import DailyActivityScope, ExportType
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def evaluate(case: dict[str, Json]) -> Json:
    scope = TypeAdapter(DailyActivityScope).validate_python(case["scope"])
    function = str(case["function"])
    limit = int(str(case.get("limit", case.get("batch_size", 0))))
    if function == "search":
        query = source.build_key_search_sql(scope, search=str(case["search"]), limit=limit)
    elif function == "model":
        query = source.build_model_top_keys_sql(
            scope,
            model_group=str(case["model_group"]),
            by_model_group=bool(case["by_model_group"]),
            limit=limit,
        )
    elif function == "leakage":
        query = source.build_cache_leakage_keys_sql(scope, limit=limit)
    elif function == "export":
        after_value = case.get("after")
        after = (
            None
            if after_value is None
            else TypeAdapter(source.ExportCursor).validate_python(after_value)
        )
        query = source.build_export_sql(
            scope,
            export_type=ExportType(str(case["export_type"])),
            after=after,
            batch_size=limit,
        )
    else:
        raise ValueError("unknown function")
    return {"sql": " ".join(query.sql.split()), "params": cast(Json, list(query.params))}


def run(input_value: dict[str, Json]) -> Json:
    cases = cast(dict[str, dict[str, Json]], input_value["cases"])
    output: dict[str, Json] = {}
    for name, case in cases.items():
        try:
            output[name] = {"query": evaluate(case), "error": None}
        except (ValueError, OverflowError) as error:
            output[name] = {"query": None, "error": str(error)}
    return output
