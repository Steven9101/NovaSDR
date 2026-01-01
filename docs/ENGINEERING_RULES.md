# Engineering Rules

This document defines the engineering standards for NovaSDR.

## Core philosophy

Write code as if it will be read, audited, extended, and maintained for 5-10 years.

- Prefer explicitness over cleverness.
- Prefer boring correctness over abstraction.
- Avoid speculative architecture.
- Avoid framework-driven design.

## Comments & documentation

Documentation lives in `docs/` and must be updated alongside code changes.

Inline comments are allowed only for:

- Non-obvious invariants
- Security decisions
- Edge-case reasoning
- Architectural constraints

Do not comment obvious code.

## Code structure & layout

- Follow idiomatic conventions of the language.
- Each file has a clear responsibility.
- Avoid god files.
- Avoid deep module trees without a clear reason.

Functions should:

- Do one thing.
- Be testable in isolation.
- Have predictable side effects.
- Avoid global state unless unavoidable.

## Naming

Names must be:

- Precise
- Boring
- Unambiguous

Avoid abbreviations unless universally understood. Types, functions, and variables must be readable in isolation.

## Error handling

Errors are first-class:

- No silent failures.
- No ignored results.
- No panics in production paths.

Errors must be actionable and must not leak sensitive information.

## Logging & observability (backend)

- Never use ad-hoc logging (`print`, `println`, etc.).
- Use structured logging (`tracing`) only.

Logs must be:

- Structured
- Machine-readable
- Human-readable

Mandatory log events:

- Startup and shutdown
- Version/build metadata
- Security events
- Admin actions
- Failures and recoveries

Never log passwords, tokens, or secrets.

## Startup identity & versioning

The application must present a clean startup banner that includes:

- Application name
- Version
- Build metadata (if available)
- OS
- Timestamp

Version must come from the build system, not hardcoded strings.

## Security (non-negotiable)

- Security is enforced server-side only.
- Never trust frontend validation.
- Authentication and authorization must be explicit.

Passwords:

- Never stored plaintext
- Never logged
- Always hashed with modern algorithms

Sessions:

- Expiring
- HttpOnly
- SameSite=strict

Normal users must never be able to mutate state. Admin privileges must be explicit and audited.

## API design

- Use RESTful semantics (correct verbs/status codes).
- Stateless requests.
- Explicit versioning.
- Validate inputs; outputs must be deterministic and documented.

## Frontend standards

UI philosophy:

- Deliberate, calm, modern
- No generic layouts or template styling

Component library:

- Use shadcn/ui components exclusively.
- Extend them carefully and consistently.

Data handling:

- Frontend consumes backend APIs (no business logic in UI components).
- No sensitive data in `localStorage`.
- Session state via cookies only.

Loading/empty states must be designed and polished (skeletons, clear messaging).

## Testing standards

Tests are mandatory for:

- Core logic
- Security boundaries
- APIs

Avoid snapshot spam and flaky tests. Broken tests are treated as broken code.

## Dependencies

Minimize dependencies. Each dependency must be justified and should be well-maintained and widely used.

## Explicitly forbidden

- TODOs in production code
- Commented-out code
- Debug hacks
- Magic constants
- "We'll fix this later" implementations
