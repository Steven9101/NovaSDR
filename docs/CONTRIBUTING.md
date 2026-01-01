# Contributing

NovaSDR uses a simple two-branch model.

## Branches

- `main`: stable releases
- `develop`: day-to-day development

## Development flow

1. Fork this repository on GitHub.
2. Branch from `develop` in your fork (feature branch).
3. Open a PR from your fork into `develop` for review.
4. Merge into `develop` after CI passes.
5. When a release is ready, open a PR from `develop` into `main`.

> [!IMPORTANT]
> If you want to make changes, **do not create a separate "new" repository**.
> Please **fork** and open a PR so maintainers and upstream attribution remain visible.

## CI behavior

CI runs on pull requests targeting `main` or `develop` and validates:

- formatting (`cargo fmt --check`)
- unit tests (`cargo test`)
- lint (`cargo clippy --workspace --all-targets -- -D warnings`)
- compile-only check for SoapySDR + clFFT (`cargo check -p novasdr-server --features "soapysdr,clfft"`)

## Releases

Releases are created from `main` using a version tag.

1. Merge `develop` into `main`.
2. Tag the merge commit (example):

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. The `release` workflow builds and publishes artifacts for:
   - Linux (x86_64)
   - Windows (x86_64)
   - Raspberry Pi (aarch64 and armv7)
