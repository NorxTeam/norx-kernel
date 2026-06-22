# Contributing

## Branches

- Linear development uses version branches: `0.1`, `0.2`, `1.9.3`.
- Side work may use `version-feature` branches, for example `0.1-fs` or
  `0.1-ci`.
- Do not create feature branches for small linear changes.

## Releases

- Release pull requests are named `<version>_RELEASE`, for example
  `0.1_RELEASE`.
- Regular commits use concise imperative messages, for example
  `Add process runtime accounting`.

## Checks

Run before opening a pull request:

```sh
cargo fmt --check
cargo build --target x86_64-unknown-uefi
cargo build --target aarch64-unknown-uefi
```
