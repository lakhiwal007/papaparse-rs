# Security policy

Please report vulnerabilities (for example a panic, hang or excessive memory use on crafted
input) privately through GitHub Security Advisories on this repository instead of opening a
public issue. We aim to respond within 7 days.

## Hardening

- No `unsafe` code (`unsafe_code = "forbid"`); the only runtime dependency is `memchr`.
- Parsing time is linear in the input size for all inputs, including adversarial ones that
  make PapaParse quadratic.
- Continuous checks: coverage-guided fuzzing (`fuzz/`), differential fuzzing against
  PapaParse, Miri, `cargo deny` (RustSec advisories, licenses, sources), heap accounting.

Known limitation shared with PapaParse: each malformed quote produces an error object, so
crafted input can make `errors` large (one entry per few input bytes). Cap the input size
if you parse untrusted data in a memory-constrained environment.
