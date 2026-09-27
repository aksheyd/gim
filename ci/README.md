# Style checks

Reusable checks for Rust repos: file length, no comments, and a unit-test cap.
Integration tests live in `tests/` and are not counted against the cap.

## Local

```
./ci/check.sh
```

## This repo

`.github/workflows/ci.yml` runs the composite action at `./ci`.

## Another repo

Copy `ci/` or point a workflow at this action once it is published:

```yaml
- uses: aksheyd/rust-style-checks@v1
  with:
    max-lines: "500"
    max-unit-tests: "2"
```

Until that action exists, vendor this directory and `uses: ./ci`.

## Knobs

| Input | Env | Default |
|---|---|---|
| `max-lines` | `MAX_LINES` | 500 |
| `max-unit-tests` | `MAX_UNIT_TESTS` | 2 |
| `src-root` | `SRC_ROOT` | `src` |
