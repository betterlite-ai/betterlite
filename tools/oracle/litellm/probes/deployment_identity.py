from typing import Protocol

from litellm.exceptions import BadRequestError
from litellm.router import Router
from pydantic import JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class IdSource(Protocol):
    @staticmethod
    def generate_model_id(model_group: str, litellm_params: dict[str, JsonValue]) -> str: ...


ID_SOURCE: IdSource = Router


class Output(Strict):
    id: str
    created: bool
    raised: bool


class DeploymentIdentity(Operation):
    model_name: str
    litellm_params: dict[str, JsonValue]
    ignore_invalid_deployments: bool

    @override
    def run(self) -> Output:
        generated = ID_SOURCE.generate_model_id(self.model_name, dict(self.litellm_params))
        try:
            router = Router(
                model_list=[{"model_name": self.model_name, "litellm_params": dict(self.litellm_params)}],
                ignore_invalid_deployments=self.ignore_invalid_deployments,
            )
        except (ValueError, TypeError, BadRequestError):
            return Output(id=generated, created=False, raised=True)
        return Output(id=generated, created=len(router.get_model_list() or []) == 1, raised=False)
