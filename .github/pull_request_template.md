## Summary

<!-- What does this change, and why? Link related issues with "Fixes #123". -->

## Checklist

- [ ] The PR title follows [Conventional Commits](https://www.conventionalcommits.org/) (`feat: ...`, `fix: ...`)
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes
- [ ] `cargo test --workspace --locked` passes
- [ ] `CHANGELOG.md` has an entry under "Unreleased" for user-visible changes
- [ ] Documentation is updated if flags, files or the D-Bus interface changed
