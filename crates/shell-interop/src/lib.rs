//! shell-interop: every Win32 / COM call the app makes lives here.
//!
//! Most Shell APIs require a single-threaded apartment, so work is executed on small pools
//! of STA worker threads ([`sta::StaPool`]). The context menu is the exception: it must run
//! on the thread that owns the window (the Tauri main thread).

pub mod clipboard;
pub mod context_menu;
pub mod icons;
pub mod info;
pub mod ops;
pub mod props;
pub mod sta;
pub mod system;
pub mod util;

pub use sta::StaPool;
