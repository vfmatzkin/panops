# Contributing to panops

The workflow contract lives in [`AGENTS.md`](AGENTS.md). Read it before opening a PR. It covers slice discipline, the debt rule, label taxonomy, and the merge checklist.

## Before you start

panops ships in thin vertical slices. One slice = one PR = one review gate. Slice N+1 doesn't start until slice N is merged. Check the [project board](https://github.com/users/vfmatzkin/projects/1) to see the active slice and what's open inside it.

If you want to work on something, comment on an existing issue first. Don't open a PR for anything outside the active slice's plan without discussing it.

## Filing issues

Use the templates at `.github/ISSUE_TEMPLATE/`. Apply the canonical labels (`type:*`, `area:*`, `severity:*` where applicable). Bugs need repro steps. Tech debt needs a concrete proposal, not "this could be better".

## Pull requests

- Keep the change scoped to one slice's plan.
- Run `cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` before pushing.
- Single-line commit messages, imperative mood, under 72 chars. No co-author attribution.
- Wait for CI and Copilot's review. Address every inline comment (fix or reply with reasoning), then resolve the thread.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Report issues to panops@tzk.ar.

## License

By contributing you agree your work is licensed under the MIT License (see [LICENSE](LICENSE)).
