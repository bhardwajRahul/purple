# Contributing

Thanks for taking the time. Bug reports, fixes and features are all welcome.

## Before you open a pull request

Run the full gate:

```
./scripts/precommit.sh
```

Everything CI checks runs there: formatting, clippy with warnings denied, the
test suite, the MSRV build, the design and message conventions, the visual
goldens and the asset pipeline. Green locally means green on CI.

## Tests

Every new code path needs a test. Unit tests for logic, render tests for UI.
Tests that write to disk use `tempfile::tempdir()` for isolation, never a path
built from the process id alone.

Code behind `std::process::exit` cannot be reached from a unit test. Spawn the
real binary against a temp `HOME` instead and assert on the exit status plus
what lands on disk. `tests/provider_add_e2e.rs` is the pattern to copy.

The suite has a known race: run it with `--test-threads=1` if a batch of
render tests fails for no apparent reason. Some of them read global theme
state that a parallel test is allowed to change.

## Changing the UI

Every screen has a golden baseline under `tests/visual_golden/`. After an
intentional visual change:

```
./scripts/update-golden.sh
git diff tests/visual_golden/
```

Review the diff before staging it. Never hand-edit a golden file.

A new `Screen` variant also needs a `visual_<name>` test in
`src/visual_regression_tests.rs`.

## Text

Everything that ships is in English: code, comments, commit messages, docs and
user-facing text.

User-facing strings live in the `messages` module, never inline in handler,
CLI or UI code. `./scripts/check-messages.sh` enforces it.

## Commits

Conventional Commits, so `feat:`, `fix:`, `change:`, `chore:`, `docs:` or
`refactor:` in the subject. Stage specific files rather than `git add -A`.
Bumping a version in `Cargo.toml` means staging `Cargo.lock` with it, since CI
builds with `--locked`.

## Providers

Every cloud provider API call needs a deserialization or HTTP test. Adding or
changing one means adding a golden fixture under `tests/api_contracts/` plus a
contract test in `tests/contract_snapshots.rs`. Changing which fields get
deserialized means updating the OpenAPI fragments under
`tests/api_contracts/openapi/`.
