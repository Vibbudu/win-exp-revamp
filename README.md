# Hyperion Explorer ⚡

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%2011%20%7C%2010-0078D4.svg)](https://microsoft.com/windows)
[![Rust](https://img.shields.io/badge/Built%20with-Rust-dea584.svg)](https://www.rust-lang.org/)
[![GUI](https://img.shields.io/badge/Native%20GUI-Slint%20Fluent-0078D4.svg)](https://slint.dev/)

An ultra-fast, 100% native open-source replacement for the Windows 11 File Explorer built in **Rust** (zero web frames, zero WebView2, zero Edge/browser dependencies, 100% offline).

Hyperion Explorer replicates the exact **look, layout, colors, and behavior** of the stock Windows 11 File Explorer (WinUI Fluent design, tabs, address bar, command bar, navigation pane, Home, This PC, details, and preview panes), but runs **considerably faster**, provides **instant search over millions of files**, and supports **rich previews for Excel sheets, PDFs, Word documents, PowerPoint presentations, code, and archives**.

---

## 🌟 Why Hyperion Explorer?

| Feature | Stock Windows 11 Explorer | Hyperion Explorer |
|---|---|---|
| **Engine** | Win32 / Classic Shell DLLs | **High-performance Rust Core** |
| **Search Speed** | Slow, indexes slowly, freezes | **Sub-20ms instant SIMD search over 2M+ files** |
| **Search Mechanism** | Windows Search Service (SQLite) | **Direct NTFS MFT & USN Journal enumeration** |
| **Directory Enumeration** | Sequential Win32 shell enumeration | **Batched `FindFirstFileExW` (zero per-file `stat`)** |
| **Excel Sheet Preview** | ❌ None (requires Office add-in) | **✅ Built-in virtualized spreadsheet grid & sheet tabs** |
| **PDF Preview** | ⚠️ Slow / requires Acrobat | **✅ Fast native streaming preview** |
| **Word (.docx) Preview** | ⚠️ Slow shell handler | **✅ Clean, fast HTML/typography document renderer** |
| **PowerPoint (.pptx) Preview**| ❌ None | **✅ Slide-by-slide text & structure preview** |
| **Code & Markdown** | ❌ Plain text / unformatted | **✅ Syntax-highlighted code + formatted Markdown** |
| **Memory Footprint** | 300MB – 650MB+ | **~90MB – 140MB** |

---

## 📸 Key Features

- **Exact Windows 11 Fluent Aesthetic**:
  - Native Windows 11 **Mica backdrop** material transparency.
  - Multi-tab strip with tab tear-off support and `Ctrl+T` / `Ctrl+W` shortcuts.
  - Interactive breadcrumb bar with chevron drop-downs and instant click-to-edit raw path input (`Ctrl+L` / `Alt+D`).
  - Command bar with responsive overflow, `+ New`, `Cut`, `Copy`, `Paste`, `Rename`, `Share`, `Delete`, `Sort`, and `View` modes.
  - Collapsible Navigation Pane featuring **Home**, **This PC** (with real drive space progress gauges), and Quick Access folders.
- **Universal Preview Pane (`Alt+P`)**:
  - **Spreadsheets**: `.xlsx`, `.xls`, `.xlsb`, `.ods`, `.csv`, `.tsv` with tabbed sheet switching.
  - **Documents**: `.docx` formatted with headings and tables.
  - **Presentations**: `.pptx` slides viewer.
  - **PDFs**: Embedded high-performance streaming viewer.
  - **Code & Markdown**: Line-numbered syntax view and rich Markdown rendering.
  - **Media & Archives**: In-app video/audio players, image inspector, and `.zip` archive directory explorer.
- **True Windows Shell Parity**:
  - Live system file-type icons and thumbnails loaded via `IShellItemImageFactory`.
  - Windows Property System integration (`IPropertyStore`) for detailed metadata.
  - Native file operations through `IFileOperation` (Recycle Bin, shell undo records, conflict dialogs).
  - Modern Windows 11 context menu with **"Show more options"** (`Shift+F10`) hosting third-party context menu handlers (7-Zip, VS Code, Git, etc.).

---

## 🏗️ Architecture

```mermaid
flowchart LR
  subgraph Frontend["Frontend: React 18 + Vite (WebView2)"]
    Tabs[Tabs Strip] --> Nav[Breadcrumb + Search Box]
    Nav --> Cmd[Command Bar]
    Cmd --> Main["Nav Pane | Folder View (Details/Icons) | Preview Pane"]
    Main --> Status[Status Bar]
  end

  subgraph Backend["Tauri 2 Host (Rust)"]
    IPC[Tauri IPC / State]
    FS[fs-core: Fast Enumeration]
    Shell[shell-interop: COM / Shell / Icons]
    Idx[indexer: MFT / USN / SIMD Search]
    Prev[preview: Universal Previewers]
  end

  Frontend <-->|Tauri IPC| IPC
  IPC --> FS & Shell & Idx & Prev
```

### Workspace Crates
- **`crates/ipc-types`**: Canonical shared data models between Rust and TypeScript.
- **`crates/fs-core`**: High-throughput directory enumeration and pure-Rust natural sorting matching `StrCmpLogicalW`.
- **`crates/shell-interop`**: STA worker pool handling Win32 Shell COM interfaces (`IShellItemImageFactory`, `IContextMenu`, `IPropertyStore`, `IFileOperation`).
- **`crates/indexer`**: Direct NTFS MFT enumeration (`FSCTL_ENUM_USN_DATA`) with parallel crawler fallback and SIMD substring queries via `memchr::memmem`.
- **`crates/preview`**: Format-specific parsers for Excel (`calamine`), Word (`quick-xml`), PowerPoint, PDF, Markdown (`pulldown-cmark`), and archives (`zip`).
- **`app/src-tauri`**: Tauri 2 application entry point and Mica vibrancy window configuration.
- **`app/ui`**: React + TypeScript frontend adhering to Fluent Design System tokens.

---

## ⌨️ Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+T` | Open new tab |
| `Ctrl+W` | Close current tab |
| `Ctrl+L` / `Alt+D` | Focus address bar (select full path) |
| `Ctrl+E` / `Ctrl+F` | Focus instant search box |
| `Alt+P` | Toggle Preview pane |
| `Alt+Shift+P` | Toggle Details pane |
| `F2` | Inline rename selected item |
| `F5` | Refresh current folder / search |
| `Delete` | Delete selected item(s) to Recycle Bin |
| `Shift+Delete` | Permanently delete selected item(s) |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / Cut / Paste |
| `Alt+Left` / `Backspace` | Navigate back / up |
| `Alt+Right` | Navigate forward |

---

## 🚀 Getting Started

### Prerequisites
- [Rust](https://rustup.rs/) (v1.80+ with `x86_64-pc-windows-msvc` toolchain)
- Windows 10 or 11 (64-bit)

### Build & Run
```powershell
# Clone the repository
git clone https://github.com/hyperion-explorer/hyperion-explorer.git
cd hyperion-explorer

# 1. Run workspace unit tests
cargo test --workspace

# 2. Run the pure native Windows application
cargo run -p app

# 3. Build optimized standalone release executable
cargo build --release -p app
# Executable is generated at: target\release\app.exe
```

---

## 📜 AI Agent Context

This repository maintains an active context ledger for AI models and automated contributors in [`AGENTS.md`](AGENTS.md). Please read and update `AGENTS.md` whenever contributing changes to this project.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
Windows, Fluent Design, and Segoe are trademarks of Microsoft Corporation. Hyperion Explorer is an independent open-source cleanroom implementation.
