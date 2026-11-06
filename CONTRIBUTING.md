# Contributing to Hyperion Explorer

Thanks for checking out the project. Here are a few simple guidelines to keep changes clean and easy to review.

## Guidelines
- No proprietary Microsoft binaries or decompiled assets. All shell integration must use documented Win32, COM, or WinRT APIs.
- Keep the app 100% native. No web runtimes, WebView2, or localhost servers.
- Keep dependencies lean. Prefer the Win32 API or pure Rust crates with small build footprints.

## Development Setup
Make sure you have the MSVC toolchain installed:

```powershell
rustup default stable-x86_64-pc-windows-msvc
```

Check that the project builds and all tests pass:

```powershell
cargo check --workspace
cargo test --workspace
```

## Commit Conventions
We use simple, lowercase scoped commit messages:

- `app/ui: add compact view toggle`
- `indexer: fix search query lag on large folders`
- `shell-interop: add mouse side button handling`
- `readme: update build instructions`

Keep commit summaries short, human, and in lowercase.

## Pull Requests
1. Fork the repo and create a branch from `main`.
2. Ensure `cargo test --workspace` passes cleanly.
3. Keep PRs focused on a single feature or bug fix.
