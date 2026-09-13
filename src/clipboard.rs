//! Clipboard providers: an in-memory slot that always works and a system
//! clipboard that mirrors into it.

pub trait Clipboard {
    /// Current clipboard text; `None` when empty or unavailable.
    fn get(&mut self) -> Option<String>;
    /// Stores `text`; empty strings are never written.
    fn set(&mut self, text: &str);
}

#[derive(Debug, Default)]
pub struct MemClipboard(Option<String>);

impl Clipboard for MemClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.clone()
    }

    fn set(&mut self, text: &str) {
        if !text.is_empty() {
            self.0 = Some(text.to_string());
        }
    }
}

#[cfg(feature = "clipboard")]
pub struct SystemClipboard {
    inner: arboard::Clipboard,
    mirror: MemClipboard,
}

#[cfg(feature = "clipboard")]
impl SystemClipboard {
    /// `None` when no system clipboard is reachable (headless, SSH).
    pub fn new() -> Option<Self> {
        let inner = arboard::Clipboard::new().ok()?;
        Some(SystemClipboard {
            inner,
            mirror: MemClipboard::default(),
        })
    }
}

#[cfg(feature = "clipboard")]
impl Clipboard for SystemClipboard {
    fn get(&mut self) -> Option<String> {
        match self.inner.get_text() {
            Ok(text) if !text.is_empty() => Some(text),
            _ => self.mirror.get(),
        }
    }

    fn set(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.mirror.set(text);
        let _ = self.inner.set_text(text);
    }
}

/// The system clipboard when available, else the in-memory slot.
pub fn default_clipboard() -> Box<dyn Clipboard> {
    system_clipboard().unwrap_or_else(|| Box::new(MemClipboard::default()))
}

#[cfg(feature = "clipboard")]
fn system_clipboard() -> Option<Box<dyn Clipboard>> {
    let system = SystemClipboard::new()?;
    Some(Box::new(system))
}

#[cfg(not(feature = "clipboard"))]
fn system_clipboard() -> Option<Box<dyn Clipboard>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_clipboard_ignores_empty_writes() {
        let mut c = MemClipboard::default();
        assert_eq!(c.get(), None);
        c.set("abc");
        c.set("");
        assert_eq!(c.get(), Some("abc".to_string()));
    }
}
