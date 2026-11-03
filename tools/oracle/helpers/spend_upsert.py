import json
from typing import Literal, TypeAlias, cast

from litellm.proxy.db import autorouter_session_rollup as turns
from litellm.proxy.db import daily_spend_bulk_upsert as daily
from litellm.proxy.db.autorouter_session_rollup import (
    _upsert_params,  # pyright: ignore[reportPrivateUsage]
)
from pydantic import TypeAdapter, ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Entity: TypeAlias = Literal["user", "team", "org", "tag", "end_user", "agent"]


def probe(step: dict[str, Json]) -> Json:
    kind = step["kind"]
    if kind == "extract":
        try:
            ids = daily.daily_spend_entity_ids(
                cast(dict[str, object], json.loads(cast(str, step["payload_json"]))),
                cast(Entity, step["entity"]),
            )
            return {"ids": list(ids)}
        except ValidationError:
            return {"invalid": True}
    if kind == "daily":
        table = daily.DAILY_SPEND_TABLES[cast(Entity, step["entity"])]
        groups = daily.merge_by_conflict_key(
            table, cast(list[dict[str, object]], json.loads(cast(str, step["rows_json"])))
        )
        sql, params = daily.build_bulk_upsert(table, groups)
        return {"groups": cast(Json, groups), "sql": " ".join(sql.split()), "params": list(params)}
    if kind == "turn":
        adapter = TypeAdapter(turns.AutoRouterTurnTransaction)
        transaction = adapter.validate_python(step["transaction"])
        params = cast(list[Json], list(_upsert_params(transaction)))
        return {"sql": " ".join(turns.UPSERT_AUTOROUTER_SESSION_SQL.split()), "params": params}
    raise ValueError("unknown spend upsert operation")


def run(input_value: dict[str, Json]) -> Json:
    steps = cast(dict[str, dict[str, Json]], input_value["steps"])
    return {name: probe(step) for name, step in steps.items()}
