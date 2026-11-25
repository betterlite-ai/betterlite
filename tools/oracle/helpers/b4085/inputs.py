from typing import Literal, TypedDict


class AuthInput(TypedDict):
    token: str
    spend: float
    max_budget: float | None
    team_id: str | None


class TeamInput(TypedDict):
    team_id: str
    max_budget: float
    spend: float


class ArmInput(TypedDict):
    router: str
    shadow: Literal["reply", "empty"]


class Input(TypedDict):
    operation: Literal["pipeline"]
    auth: AuthInput | None
    key_spend: float | None
    team: TeamInput | None
    team_spend: float | None
    database: bool
    pipeline_error: bool
    max_budget: float | None
    job_spend: float | None
    stats_spend: str | None
    judge_reply: Literal["a", "b", "tie", "invalid"]
    arms: list[ArmInput]
