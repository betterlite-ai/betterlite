from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class AffinityClaims(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "affinity_claims is not built")
