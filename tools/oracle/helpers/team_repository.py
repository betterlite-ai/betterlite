import json
from typing import TypedDict

from litellm.models.team import LiteLLM_TeamTable
from litellm.repositories.base_repository import DbRecord
from litellm.repositories.team_repository import TeamRepository
from pydantic import JsonValue, TypeAdapter, ValidationError

VALIDATOR_FIELDS = (
    "metadata",
    "aliases",
    "config",
    "permissions",
    "model_max_budget",
    "model_aliases",
    "router_settings",
    "budget_limits",
)
MODEL_DUMP = TypeAdapter(dict[str, JsonValue])


class Case(TypedDict):
    records: list[dict[str, JsonValue] | None]


class Input(TypedDict):
    cases: dict[str, Case]


class ModelOutcome(TypedDict):
    model: dict[str, JsonValue] | None


class ErrorOutcome(TypedDict):
    error: str


class Repository(TeamRepository):
    def convert(self, record: DbRecord | None) -> LiteLLM_TeamTable | None:
        return super()._to_model(record)


def model_label(error: ValidationError) -> str:
    message = str(error)
    for field in VALIDATOR_FIELDS:
        expected = f"Field {field} should be a valid dictionary"
        if expected in message:
            return expected
    return "invalid model"


def convert(
    repository: Repository, record: dict[str, JsonValue] | None
) -> ModelOutcome | ErrorOutcome:
    try:
        model = repository.convert(record)
    except json.JSONDecodeError:
        return {"error": "invalid json"}
    except ValidationError as error:
        return {"error": model_label(error)}
    if model is None:
        return {"model": None}
    return {"model": MODEL_DUMP.validate_json(model.model_dump_json())}


def run(input_value: Input) -> dict[str, list[ModelOutcome | ErrorOutcome]]:
    repository = Repository(None)
    return {
        name: [convert(repository, record) for record in case["records"]]
        for name, case in input_value["cases"].items()
    }
