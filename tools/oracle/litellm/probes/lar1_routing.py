from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class Lar1Routing(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "lar1_routing is not built")
