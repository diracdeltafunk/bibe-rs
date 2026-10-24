<!-- What does this change, and why? Link the issue if there is one. -->

- [ ] Output, messages and exit codes match biber 2.22, or the difference has a `DC-NNN` entry in `docs/compat.md`
- [ ] Tests or corpus cases cover the change (`cargo xtask corpus` passes; new passing cases added with `--update-passing`)
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] For performance changes: `tools/bench/run.py` before/after (all outputs `ok`)
- [ ] No btparse C source was consulted for changes to the `btparse` crate (see CONTRIBUTING.md)
