from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class AutomaticRouterPipeline(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "automatic_router_pipeline is not built")
