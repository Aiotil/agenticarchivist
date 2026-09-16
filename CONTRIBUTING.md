# Contributing

Thanks for your interest in AgenticArchivist.

## Before you start

The project is in early design. For anything beyond a small fix, open an issue first so we can agree on the approach.

## Licensing of contributions

AgenticArchivist is dual licensed under MIT OR Apache-2.0. Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion is dual licensed the same way, without any additional terms or conditions.

## Developer Certificate of Origin

Every commit must be signed off, certifying that you wrote the change or otherwise have the right to submit it under the project's licences, as described in the [Developer Certificate of Origin 1.1](https://developercertificate.org/).

Add the sign-off with `git commit -s`, which appends a line like:

```
Signed-off-by: Your Name <you@example.com>
```

## Dependencies

New dependencies must meet the [dependency policy](docs/dependency-policy.md): an open-source licence compatible with a permissive app that also ships on the iOS App Store, and several years of active maintenance. Explain in your pull request how a new dependency meets both.

## Code style

- Rust: `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass.
- Keep changes focused; one topic per pull request.

## Archival compatibility

Anything written into a collection folder (sidecars, catalog files, log format) is an archival format. Changes to those formats need a version bump and a migration path, and must never modify original files.
