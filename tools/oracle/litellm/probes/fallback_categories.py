from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class FallbackCategories(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "fallback_categories is not built")
