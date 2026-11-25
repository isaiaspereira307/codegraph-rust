# ABOUTME: Compares per-function risk scores between a CRG dump and Rust detect_changes.
# ABOUTME: Shared by scripts/parity/changes/compare_changes.py and the P1 unit tests.
"""Small, side-effect-free helper used by the change-parity comparator."""


def equal_risk_count(crg_risks, rust_risks, tolerance=0.001):
    """Return (equal, common) for functions present on both sides.

    Keys must already be root-relative qualified names; a missing function is not a match.
    """
    common = set(crg_risks) & set(rust_risks)
    equal = sum(
        1
        for key in common
        if abs(float(crg_risks[key]) - float(rust_risks[key])) <= tolerance
    )
    return equal, len(common)
