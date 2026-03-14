# Contributing to G-Claw

Thanks for your interest in G-Claw! Here's how to get started.

## Development Setup

1. **Rust toolchain** (1.85+):
   ```bash
   rustup update stable
   ```

2. **Clone and build** (core, no native deps):
   ```bash
   git clone https://github.com/Dawash/G-Claw.git
   cd G-Claw
   cargo check
   cargo test
   ```

3. **Full build** (requires LLVM/libclang):
   ```bash
   # Linux: sudo apt install libclang-dev libasound2-dev
   # macOS: brew install llvm
   # Windows: Install LLVM, set LIBCLANG_PATH
   cargo build --features full
   ```

4. **Download models** (for runtime testing):
   ```bash
   python scripts/download_models.py
   ```

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix all warnings
- Keep functions focused and small
- Add doc comments to public items
- No `unwrap()` on fallible operations in non-test code (use `?` or `context()`)

## Testing

```bash
# Run all tests
cargo test

# Run a specific crate's tests
cargo test -p gclaw-ipc
cargo test -p gclaw-config
cargo test -p gclaw-voice
```

When adding new functionality, include tests. Aim for:
- Unit tests for pure logic (ring buffers, fuzzy matching, WAV parsing)
- Integration tests for IPC roundtrips
- Edge cases: empty inputs, malformed data, timeouts

## Pull Requests

1. Fork the repo and create a feature branch
2. Make your changes with clear commit messages
3. Ensure `cargo test`, `cargo fmt --check`, and `cargo clippy` pass
4. Open a PR with:
   - **Summary**: What changed and why
   - **Test plan**: How you verified it works

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the system overview.

Key crates:
- `gclaw-ipc` — Shared protocol, codec, transport (touch this carefully)
- `gclaw-config` — Config loading + Fernet crypto
- `gclaw-voice` — Audio pipeline (capture, VAD, STT, TTS, wake word, barge-in)

## Reporting Issues

Use [GitHub Issues](https://github.com/Dawash/G-Claw/issues) with:
- OS and Rust version
- Steps to reproduce
- Expected vs actual behavior
- Logs (run with `RUST_LOG=debug`)

## License

By contributing, you agree that your contributions are licensed under the MIT License.
