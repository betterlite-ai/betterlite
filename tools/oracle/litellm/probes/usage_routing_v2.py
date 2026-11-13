from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class UsageRoutingV2(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "usage_routing_v2 is not built")
