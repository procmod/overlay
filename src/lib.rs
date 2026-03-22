//! Cross-platform game overlay rendering with transparent, click-through windows.
//!
//! `procmod-overlay` creates a transparent overlay window on top of a target game window
//! and provides an immediate-mode 2D drawing API for shapes, text, and game HUD elements.

mod color;
mod error;
#[allow(dead_code)]
mod font;
#[allow(dead_code)]
mod vertex;

#[cfg(target_os = "windows")]
mod overlay;
#[cfg(target_os = "windows")]
mod renderer;
#[cfg(target_os = "windows")]
mod window;

pub use color::Color;
pub use error::{Error, Result};

#[cfg(target_os = "windows")]
pub use overlay::Overlay;
#[cfg(target_os = "windows")]
pub use window::OverlayTarget;
