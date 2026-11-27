from typing import Final, TypedDict

from litellm.proxy.management_helpers.bulk_user_deletion import _forbidden, _team_not_found  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/bulk_user_deletion.py#L120-L134
from pydantic import JsonValue, TypeAdapter


class Problem(TypedDict):
    type: str
    title: str
    status: int
    detail: str
    allowed: list[str] | None


class Answer(TypedDict):
    problem: Problem
    message: str


class Call(TypedDict, total=False):
    team_not_found: str
    forbidden: str


class Input(TypedDict):
    calls: list[Call]


PROBLEM: Final[TypeAdapter[Problem]] = TypeAdapter(Problem)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def answer(call: Call) -> Answer:
    team_id = call.get("team_not_found")
    detail = call.get("forbidden")
    if team_id is not None:
        exception = _team_not_found(team_id)
    elif detail is not None:
        exception = _forbidden(detail)
    else:
        raise ValueError("a call names team_not_found or forbidden")
    problem = PROBLEM.validate_json(exception.problem.model_dump_json())
    return {"problem": problem, "message": str(exception)}


def run(input_value: Input) -> JsonValue:
    answers: list[Answer] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
