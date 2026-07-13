# Final fix report

## TDD evidence

Red command: `cargo test monitor::runtime::tests::`

Before production changes, the focused suite failed as expected:

```text
rediscovered_lockout_does_not_create_a_new_revision_or_bypass_exhaustion: left 8, right 7
older_rediscovery_does_not_replace_a_materially_newer_target: left 10, right 9
preview_byte_cap_aligns_to_utf8_character_boundaries: panicked: end byte index 1048576 is not a char boundary
test result: FAILED. 11 passed; 3 failed
```

Green command: `cargo test monitor::runtime::tests::`

```text
test result: ok. 16 passed; 0 failed
```

## Changes

- Treat lockout revision as a target generation: identical and older observations are no-ops; a newer target advances the revision.
- Align mmap overlap starts and preview offsets forward to UTF-8 boundaries, and floor the byte-capped preview end to a boundary.
- Keep Tokio `test-util` in dev-dependencies while using the same major-version requirement.
- Reword benchmark evidence to require a separately reviewed controlled measurement plan; no production optimization behavior was added.

## Self-review

- Strict UTF-8 validation remains in place; no lossy decoding was introduced.
- Existing exhausted revisions remain exhausted after duplicate observations.
- A newer target remains eligible because its revision changes naturally.
- Changes are limited to the reported findings and regression coverage.

## Validation

Final full gate outputs are recorded in the task handoff after running formatting, tests, Clippy, and diff checks.
