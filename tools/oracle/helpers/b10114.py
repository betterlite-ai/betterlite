from typing import Final, TypedDict

from litellm.proxy.management_endpoints.ui_sso import (
    _cli_sso_team_detail,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L2248
    selected_cli_sso_team_detail,
)
from pydantic import JsonValue, TypeAdapter


class TeamRow(TypedDict):
    team_id: str
    team_alias: str | None
    models: list[str]
    aliases: JsonValue


class Input(TypedDict):
    team_details: JsonValue
    team_id: str | None
    row: TeamRow


class TeamDetail(TypedDict):
    team_id: str | None
    team_alias: str | None
    team_models: list[str]
    team_model_aliases: dict[str, str] | None


class Output(TypedDict):
    selected: TeamDetail | None
    detail: TeamDetail


DETAIL: Final[TypeAdapter[TeamDetail]] = TypeAdapter(TeamDetail)


def run(input_value: Input) -> Output:
    row = input_value["row"]
    detail = _cli_sso_team_detail(
        {
            "team_id": row["team_id"],
            "team_alias": row["team_alias"],
            "models": row["models"],
            "litellm_model_table": (
                None if row["aliases"] is None else {"model_aliases": row["aliases"]}
            ),
        }
    )
    selected = selected_cli_sso_team_detail(input_value["team_details"], input_value["team_id"])
    return {
        "selected": (
            None if selected is None else DETAIL.validate_json(selected.model_dump_json())
        ),
        "detail": DETAIL.validate_json(detail.model_dump_json()),
    }
