# Contributing to lmd-top

Thanks for your interest! Issues and pull requests are welcome.

## Reporting issues

- For bugs, include the output of `lmd-top --doctor` (it summarizes which exporters and
  metrics your cluster exposes) and your terminal/font if the problem is visual.
- Feature ideas are welcome too — check [ROADMAP.md](ROADMAP.md) first to see if it's
  already planned.

## Development

```bash
cargo build            # debug build
cargo test             # run tests
cargo clippy -- -D warnings
cargo fmt --check
```

Useful while developing:

- `lmd-top --render` renders one frame headlessly (size via `LMD_W`/`LMD_H`) — handy for
  layout checks, but it cannot verify colors or font glyphs; check those in a real terminal.
- `lmd-top --snapshot` prints a plain-text state dump.

## Pull requests

- Keep PRs focused — one change per PR.
- Make sure `cargo clippy -- -D warnings` and `cargo test` pass.
- The crate is **pure Rust by design**: no C-library dependencies (no OpenSSL, ring,
  pkg-config, or cmake). Please don't add crates that break this.
- `ratatui` is pinned to 0.29 and `tachyonfx` to `=0.16.0` (the last release compatible
  with monolithic ratatui 0.29) — bumping either is a coordinated change, not a routine
  dependency update.

## License

By contributing, you agree that your contributions will be licensed under the
[Apache-2.0](LICENSE) license.
