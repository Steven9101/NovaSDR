# Contributing

## Branching model

- `main` is the stable branch.
- `develop` is the integration branch for ongoing work.

## Workflow

1. Fork this repository on GitHub.
2. Create a feature branch off `develop` in your fork.
3. Open a PR from your fork into `develop`.
4. After review and CI, merge to `develop`.
5. When ready to release, open a PR from `develop` into `main`.

> [!IMPORTANT]
> If you want to make changes, **do not create a separate "new" repository**.
> Please **fork** and open a PR so the original maintainers and upstream attribution remain visible.

## CI

CI runs on pull requests targeting `main` or `develop` and checks:

- `cargo fmt --check`
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo check -p novasdr-server --features "soapysdr,clfft"` (compile-only)

## Releases

Releases are created from `main` using a version tag.

1. Merge `develop` into `main`.
2. Tag the merge commit (example):
   - `git tag v0.1.0`
   - `git push origin v0.1.0`
3. The `release` workflow builds artifacts for Linux, Windows, and Raspberry Pi targets and publishes a GitHub Release.
