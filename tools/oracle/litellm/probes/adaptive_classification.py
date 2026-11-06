from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class AdaptiveClassification(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "adaptive_classification is not built")
