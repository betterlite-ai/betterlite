from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class StrategyRouterHealth(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "strategy_router_health is not built")
