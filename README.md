<div align="center">
    <h1>Hyperion Explorer</h1>
    <p>
        A fast, offline, native Windows 11 File Explorer written in Rust.
    </p>
    <a href="https://github.com/Vibbudu/win-exp-revamp">
        github.com/Vibbudu/win-exp-revamp
    </a>
</div>

## Overview
> [!NOTE]
> Hyperion Explorer is under active development.
> Bug reports and pull requests are welcome.

Hyperion Explorer is a lightweight, 100% native replacement for the Windows 11 File Explorer. It keeps the stock WinUI Fluent design and system shell integration (native icons, Recycle Bin, context menus), but runs on a custom Rust core with instant search and built-in file previews.

No web frames, no WebView2, no Edge runtimes. Just native machine code.

## Performance vs Windows 11 Explorer

| Metric | Stock Explorer | Hyperion Explorer |
|---|---|---|
| Cold launch | ~1.2s - 2.0s | < 200ms |
| 100,000 file folder listing | 1.5s - 3.5s | ~45ms (large fetch FindFirstFileExW) |
| Search over 2M+ files | 15s - 45s (or hangs) | < 20ms (in-memory SIMD search) |
| RAM usage (idle) | 350MB - 650MB | ~45MB - 90MB |
| Excel & CSV preview | Requires Excel add-in | Built-in interactive grid |
| PDF preview | Slow shell handler | Built-in WinRT page renderer |

## Architecture
The project is organized as a Cargo workspace with dedicated crates:

### Crates
- `crates/fs-core`: Fast directory enumeration using Win32 `FindFirstFileExW` with `FIND_FIRST_EX_LARGE_FETCH` and pure-Rust natural sorting (`StrCmpLogicalW`).
- `crates/indexer`: Sub-20ms search engine. Direct NTFS MFT reading (`FSCTL_ENUM_USN_DATA`) when elevated, with a priority BFS crawler fallback and flat-array SIMD search (`memchr::memmem` + `rayon`).
- `crates/shell-interop`: STA worker pool for Windows Shell COM interfaces (`IShellItemImageFactory`, `IContextMenu`, `IPropertyStore`, `IFileOperation`), cloud sync badges, and native drag-and-drop (`SHDoDragDrop`).
- `crates/preview`: File previews for Excel (`.xlsx`, `.xls`, `.ods`, `.csv`), Word (`.docx`), PowerPoint (`.pptx`), PDF (`Windows.Data.Pdf`), code syntax detection, and `.zip` archives.
- `crates/ipc-types`: Shared data types used across crates.

### Application
- `app`: Native Windows desktop app using Slint for the UI and Win32 for windowing, system theme sync, and input hooks.

## Key Features
- Windows 11 Fluent layout (tabs, breadcrumb address bar, command bar, navigation pane, status bar).
- Compact view option (`View` -> `Show` -> `Compact view`) matching stock Explorer spacing.
- Column resizing for Name, Date modified, Type, and Size headers.
- Multi-column views: Details, Tiles, Content, List, Small, Medium, Large, and Extra Large icons.
- Instant search that updates as you type without blocking the UI thread.
- Native drag and drop (`SHDoDragDrop`) to and from external applications.
- Mouse back and forward side buttons for folder history navigation.
- Dark and light theme auto-switching based on Windows system settings.
- Google Drive and cloud folder support with cloud sync badges.
- Built-in previews for PDFs, spreadsheets, documents, presentations, and code.

## Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+T | New tab |
| Ctrl+W | Close active tab |
| Ctrl+L / Alt+D | Edit address bar |
| Ctrl+F / Ctrl+E | Focus search box |
| Alt+Left | Go back |
| Alt+Right | Go forward |
| Alt+Up / Backspace | Go up one directory |
| Alt+P | Toggle preview pane |
| Alt+Shift+P | Toggle details pane |
| F2 | Rename selected file |
| F5 / Ctrl+R | Refresh folder |
| Delete | Send to Recycle Bin |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / Cut / Paste |

## Building and Running

### Prerequisites
- Windows 10 or 11 (64-bit)
- Rust 1.80+ (MSVC toolchain: `x86_64-pc-windows-msvc`)

### Quick Start
```powershell
# Clone the repository
git clone https://github.com/Vibbudu/win-exp-revamp.git
cd win-exp-revamp

# Run unit tests
cargo test --workspace

# Run the app in debug mode
cargo run -p app

# Build an optimized release binary
cargo build --release -p app
```
The compiled binary will be at `target/release/app.exe`.

## Contributing
See [CONTRIBUTING.md](CONTRIBUTING.md) for build guidelines and coding conventions.

## Credits
- [Slint](https://slint.dev/) - Native GUI toolkit for Rust
- [Microsoft Fluent Design System](https://github.com/microsoft/fluentui) - Design tokens and icon glyphs
- [Calamine](https://github.com/tafia/calamine) - Fast spreadsheet parser

## License
MIT License. See [LICENSE](LICENSE) for details.
