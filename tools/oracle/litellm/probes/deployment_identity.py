from litellm.router import Router
from pydantic import JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


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
        generated = Router.generate_model_id(self.model_name, dict(self.litellm_params))
        router = Router(model_list=[], ignore_invalid_deployments=self.ignore_invalid_deployments)
        try:
            # litellm/router.py L8942: the deployment builder behind set_model_list, with the router's ignore setting
            deployment = router._create_deployment(
                deployment_info={},
                _model_name=self.model_name,
                _litellm_params=dict(self.litellm_params),
                _model_info={},
            )
        except Exception:
            return Output(id=generated, created=False, raised=True)
        return Output(id=generated, created=deployment is not None, raised=False)
