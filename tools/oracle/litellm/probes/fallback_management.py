from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class FallbackManagement(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "fallback_management is not built")
