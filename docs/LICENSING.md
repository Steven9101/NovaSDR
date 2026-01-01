# Licensing

## Project license

NovaSDR is licensed under the GNU General Public License, Version 3 (GPLv3).

- License text: `LICENSE`
- Attribution: `NOTICE`
 - Third-party inventory: `docs/THIRD_PARTY.md`

## Plain-English summary (not legal advice)

This section is a practical summary intended for developers. It is not legal advice.

- **GPLv3 is a "copyleft" license**: if you distribute NovaSDR (or a modified version), you must provide the corresponding source code under GPLv3 terms.
- **You still own your work**: GPLv3 does not take away your copyright in your original contributions. It sets the *rules for distribution* of the combined work.
- **Credits and provenance matter**: if NovaSDR includes or derives from upstream work, upstream attribution and license notices must remain intact.

## Why GPLv3

This repository is a continuation of PhantomSDR-plus (a fork maintained by `magicint1337`) and a port/continuation of earlier GPLv3 WebSDR work from:

- https://github.com/rhgndf

When a project is derived from GPLv3-licensed work, the resulting combined work must
remain under GPLv3-compatible terms. As a result, this Rust port is distributed under
GPLv3 as well.

## "Rewrite with inspiration" - how that interacts with GPL

People often describe ports as "rewritten from scratch" while still being guided by an existing project. From a licensing perspective, there are two important cases:

- **Derivative work (safe assumption when porting)**: if any non-trivial portion of copyrighted expression from a GPLv3 project was copied/adapted (including code, unique tables/data, or other expressive content), the result must be distributed under GPLv3-compatible terms. In that case, **GPLv3 is not optional**.
- **Independent implementation**: if the implementation is truly independent (no copying of copyrighted expression), it may be possible to license it differently. However, "inspiration" can be ambiguous in practice. For NovaSDR, we treat the project as GPLv3 to be conservative and to keep compliance straightforward.

Separately, some things are generally not protected the same way as source code (for example, high-level ideas, interoperability requirements, and many protocol details). But the line is not always obvious, so the repository policy is: **assume GPLv3 applies to the distributed combined work**.

## Copyright

You may add your own copyright notice for your original contributions while preserving
the upstream copyright notices and GPLv3 terms.

Practical guidance:

- **Add yourself as an author**: add a line to `NOTICE` (and optionally to `README.md`) naming the primary maintainer(s) and the year(s).
- **Keep upstream attribution**: do not remove upstream credits; add yours alongside them.
- **Git history is attribution**: using forks/PRs helps preserve contributor attribution and makes provenance visible.

## Project attribution expectations

The GPL requires preserving applicable copyright/license notices. Separately, NovaSDR's project policy is:

- **Do not remove attribution** from `NOTICE` or `docs/THIRD_PARTY.md`.
- **Use forks and pull requests** when making changes so maintainer and contributor attribution remains visible.

NovaSDR represents months of work. If you redistribute or build on it, please keep credits intact and visible.

## What you must do when distributing NovaSDR (high-signal checklist)

When you distribute NovaSDR binaries or source (including modified versions):

- **Include the GPL license text**: `LICENSE`.
- **Include attribution**: `NOTICE` and `docs/THIRD_PARTY.md`.
- **Provide Corresponding Source**: the preferred form for making modifications for the version you distribute.
- **Do not add extra restrictions**: you can't impose terms that conflict with GPLv3.

## Frontend licensing

The UI in `frontend/` is versioned in this repository. It is distributed with the
overall system and must be license-compatible with GPLv3 if shipped together.

When you distribute NovaSDR, ensure you also include:

- the frontend's license files, and
- the license information for third-party JavaScript dependencies (npm packages).

## Bundled generated artifacts

The repository includes generated artifacts (for example, WebAssembly modules under `frontend/src/modules/` and `frontend/src/decoders/`). If you distribute these artifacts, ensure you also distribute the corresponding license information and any required corresponding source.

## Third-party crates

The Rust backend depends on third-party crates which are licensed under their respective
terms (commonly MIT/Apache-2.0/BSD). GPLv3 permits linking to such components as long as
distribution requirements are met. For compliance workflows, capture a license report
using your preferred tooling (e.g. `cargo-deny`, `cargo-about`) as part of releases.

