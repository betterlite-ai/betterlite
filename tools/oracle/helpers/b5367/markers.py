from collections.abc import Hashable
from types import new_class
from typing import NotRequired, TypeAlias, TypedDict, cast

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None


class GuardrailSpec(TypedDict):
    name: NotRequired[str]
    default_on: bool


class LoggerSpec(TypedDict):
    logger: str
    guardrail: NotRequired[GuardrailSpec]


Entry: TypeAlias = str | LoggerSpec

CallbackValue: TypeAlias = str | CustomLogger


class Markers:
    def __init__(self) -> None:
        self.instances: dict[str, CustomLogger] = {}

    # Hashable hides type[CustomLogger] from mypy, which rejects its untyped **kwargs constructor
    def fresh_logger(self) -> CustomLogger:
        seed: Hashable = CustomGuardrail()
        name = f"Marker_{len(self.instances) + 1}"
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1019-L1026
            "CustomLogger", new_class(name, type(seed).__bases__)()
        )

    def marker(self, label: str, guardrail: GuardrailSpec | None) -> CustomLogger:
        existing = self.instances.get(label)
        if existing is not None:
            return existing
        if guardrail is None:
            created = self.fresh_logger()
        else:
            created = CustomGuardrail(
                guardrail_name=guardrail.get("name"), default_on=guardrail["default_on"]
            )
        self.instances[label] = created
        return created

    def entry(self, value: Entry) -> CallbackValue:
        match value:
            case str():
                return value
            case object():
                pass
        return self.marker(value["logger"], value.get("guardrail"))

    def entries(self, values: list[Entry] | None) -> list[CallbackValue]:
        if values is None:
            return []
        return [self.entry(value) for value in values]

    def entry_out(self, item: CallbackValue) -> Json:
        match item:
            case str():
                return item
            case object():
                pass
        for label, instance in self.instances.items():
            if instance is item:
                return {"logger": label}
        raise LookupError("a callback that the case did not create")
