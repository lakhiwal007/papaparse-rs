# Contributing

## Setup

Install [rustup](https://rustup.rs). The pinned toolchain in `rust-toolchain.toml` is fetched
on first build. Optional tools: `just`, `wasm-pack`, `cargo-deny`, `cargo-fuzz` (nightly),
`cargo-llvm-cov`.

## Before opening a PR

```sh
just check          # cargo fmt --check, clippy -D warnings (pedantic), cargo test
just conformance    # nothing in the scoreboard should regress
```

## Porting rules

- **Behavioural parity first.** When Rust and PapaParse disagree, PapaParse is right unless
  the difference is listed under "Deliberate differences" in `docs/PORTING.md`. Adding a
  difference to that list needs a reason.
- **Follow the JS structure.** Keep functions and comments recognisable next to
  `papaparse.js` so upstream changes can be ported by diffing.
- **Every behaviour gets a test.** Prefer adding an upstream fixture case; otherwise add a
  Rust test that names the upstream behaviour it mirrors.
- **No panics on any input.** Index with byte offsets only at char boundaries; the fuzz target
  (`just fuzz`) checks this.
- `unsafe` is forbidden workspace-wide.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `perf:`,
`test:`, `docs:`, `chore:`), and update `CHANGELOG.md` under *Unreleased*.
