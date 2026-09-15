//! A small terminal notes editor with no modes.
//!
//! The library holds everything except the terminal setup and the event
//! loop, so the whole editing model is testable without a TTY.

pub mod app;
pub mod buffer;
#[cfg(unix)]
pub mod client;
pub mod clipboard;
#[cfg(unix)]
pub mod daemon;
pub mod editor;
pub mod file;
pub mod keys;
pub mod mouse;
pub mod protocol;
pub mod server;
pub mod terminal;
pub mod text;
pub mod ui;
pub mod undo;
pub mod wrap;

pub use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};
