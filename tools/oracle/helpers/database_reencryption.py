from typing import Literal, TypedDict

from litellm.proxy.db import master_key_migration as migration
from litellm.proxy.db.master_key_migration import (
    Migrated,
    MigrationFailed,
    NothingToMigrate,
    describe_outcome,
    replace_ciphertexts,
)
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ReplaceInput(TypedDict):
    op: Literal["replace"]
    value: JsonValue
    replacements: dict[str, str | None]
    depth: int
    max_depth: int


class OutcomeInput(TypedDict):
    kind: Literal["salt", "no_database", "nothing", "migrated", "failed"]
    migrated: int
    remaining: int
    error_class: Literal["ValueError", "RuntimeError", "KeyError"]
    error_message: str


class OutcomeCall(TypedDict):
    op: Literal["outcome"]
    outcome: OutcomeInput


class ShapeInput(TypedDict):
    value: JsonValue
    is_json: bool


class SqlParameterCall(TypedDict):
    op: Literal["sql_parameter"]
    shapes: list[ShapeInput]


class Replacements:
    def __init__(self, values: dict[str, str | None]) -> None:
        self.values = values

    def __call__(self, value: str) -> str | None:
        return self.values.get(value)


def scripted_error(spec: OutcomeInput) -> Exception:
    match spec["error_class"]:
        case "ValueError":
            return ValueError(spec["error_message"])
        case "RuntimeError":
            return RuntimeError(spec["error_message"])
        case "KeyError":
            return KeyError(spec["error_message"])


def outcome_of(spec: OutcomeInput) -> NothingToMigrate | Migrated | MigrationFailed:
    match spec["kind"]:
        case "salt":
            return NothingToMigrate.SALT_KEY_ENCRYPTS_STORED_VALUES
        case "no_database":
            return NothingToMigrate.NO_DATABASE
        case "nothing":
            return NothingToMigrate.NOTHING_ENCRYPTED_WITH_PREVIOUS_KEY
        case "migrated":
            return Migrated(migrated=spec["migrated"], remaining=spec["remaining"])
        case "failed":
            return MigrationFailed(error=scripted_error(spec))


def run_replace(input_value: ReplaceInput) -> JsonValue:
    replaced, count = replace_ciphertexts(
        input_value["value"],
        Replacements(input_value["replacements"]),
        input_value["depth"],
    )
    return {"value": replaced, "count": count}


def parameter_shape(spec: ShapeInput) -> JsonValue:
    parameter = migration._as_sql_parameter(spec["value"], migration._SecretColumn(table="", primary_key="", column="", is_json=spec["is_json"]))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/master_key_migration.py#L159-L160
    if spec["is_json"]:
        return JSON.validate_json(parameter)
    return parameter


def run_sql_parameter(input_value: SqlParameterCall) -> JsonValue:
    return [
        parameter_shape(spec) for spec in input_value["shapes"]
    ]


def run(input_value: ReplaceInput | OutcomeCall | SqlParameterCall) -> JsonValue:
    if input_value["op"] == "replace":
        return run_replace(input_value)
    if input_value["op"] == "sql_parameter":
        return run_sql_parameter(input_value)
    return describe_outcome(outcome_of(input_value["outcome"]))
