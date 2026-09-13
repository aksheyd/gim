//! Loading and saving: line-ending and BOM detection, atomic writes, the
//! default notes path.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Files above this size are refused at load time.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

#[derive(Debug)]
pub struct Document {
    pub path: PathBuf,
    pub display_name: String,
    pub ending: LineEnding,
    pub bom: bool,
    pub saved_text: String,
    pub exists: bool,
}

impl Document {
    pub fn new(path: &Path) -> Self {
        let display_name = match path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => path.display().to_string(),
        };
        Document {
            path: path.to_path_buf(),
            display_name,
            ending: LineEnding::Lf,
            bom: false,
            saved_text: String::new(),
            exists: false,
        }
    }

    /// Whether `text` differs from the last loaded or saved text.
    pub fn is_dirty(&self, text: &str) -> bool {
        text != self.saved_text
    }

    /// Bytes as they would be written: BOM and line endings restored.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() + 3);
        if self.bom {
            out.extend_from_slice("\u{feff}".as_bytes());
        }
        match self.ending {
            LineEnding::Lf => out.extend_from_slice(text.as_bytes()),
            LineEnding::Crlf => out.extend_from_slice(text.replace('\n', "\r\n").as_bytes()),
        }
        out
    }
}

#[derive(Debug)]
pub enum LoadError {
    NotFile(PathBuf),
    TooLarge(PathBuf),
    NotUtf8(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotFile(p) => write!(f, "{}: not a regular file", p.display()),
            LoadError::TooLarge(p) => {
                let cap = MAX_FILE_BYTES / (1024 * 1024);
                write!(f, "{}: larger than {cap} MiB", p.display())
            }
            LoadError::NotUtf8(p) => write!(f, "{}: not valid UTF-8", p.display()),
            LoadError::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for LoadError {}

/// Loads `path`; a missing file yields an empty, not-yet-existing document.
pub fn load(path: &Path) -> Result<(Document, String), LoadError> {
    let mut doc = Document::new(path);
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((doc, String::new())),
        Err(e) => return Err(LoadError::Io(path.to_path_buf(), e)),
    };
    if !meta.is_file() {
        return Err(LoadError::NotFile(path.to_path_buf()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge(path.to_path_buf()));
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return Err(LoadError::Io(path.to_path_buf(), e)),
    };
    let mut text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return Err(LoadError::NotUtf8(path.to_path_buf())),
    };
    if text.starts_with('\u{feff}') {
        doc.bom = true;
        text.drain(..'\u{feff}'.len_utf8());
    }
    doc.ending = detect_ending(&text);
    if doc.ending == LineEnding::Crlf {
        text = text.replace("\r\n", "\n");
    }
    doc.exists = true;
    doc.saved_text = text.clone();
    Ok((doc, text))
}

/// CRLF when the first line ending in the text is `\r\n`.
fn detect_ending(text: &str) -> LineEnding {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => LineEnding::Crlf,
        _ => LineEnding::Lf,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveOutcome {
    pub bytes: usize,
    pub atomic: bool,
}

/// Writes `text` to the document's path: temp file next to the resolved
/// target plus rename, falling back to an in-place write only when the
/// directory refuses the temp file but the target itself is writable.
pub fn save(doc: &mut Document, text: &str) -> io::Result<SaveOutcome> {
    let Some(name) = doc.path.file_name() else {
        return Err(io::Error::other("path has no file name"));
    };
    let parent = match doc.path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&parent)?;
    // Resolve symlinks so the real file is replaced, not the link.
    let target = match fs::canonicalize(&doc.path) {
        Ok(real) => real,
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::canonicalize(&parent)?.join(name),
        Err(e) => return Err(e),
    };
    let dir = match target.parent() {
        Some(dir) => dir.to_path_buf(),
        None => PathBuf::from("."),
    };
    let existing = fs::metadata(&target).ok();
    if let Some(meta) = &existing
        && meta.permissions().readonly()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file is read-only (chmod +w to save)",
        ));
    }
    let data = doc.encode(text);
    let pid = std::process::id();
    let tmp = dir.join(format!(".{}.gim-{pid}.tmp", name.to_string_lossy()));
    let atomic = match File::create(&tmp) {
        Ok(mut file) => {
            let result = write_atomic(&mut file, &data, &tmp, &target, existing.as_ref());
            if let Err(e) = result {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
            true
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let mut file = File::options().write(true).truncate(true).open(&target)?;
            file.write_all(&data)?;
            file.sync_all()?;
            false
        }
        Err(e) => return Err(e),
    };
    doc.saved_text = text.to_string();
    doc.exists = true;
    Ok(SaveOutcome {
        bytes: data.len(),
        atomic,
    })
}

fn write_atomic(
    file: &mut File,
    data: &[u8],
    tmp: &Path,
    target: &Path,
    existing: Option<&fs::Metadata>,
) -> io::Result<()> {
    file.write_all(data)?;
    file.sync_all()?;
    if let Some(meta) = existing {
        fs::set_permissions(tmp, meta.permissions())?;
    }
    fs::rename(tmp, target)
}

/// `$GIM_NOTES`, else `$XDG_DATA_HOME/gim/notes.md`, else
/// `~/.local/share/gim/notes.md`.
pub fn default_notes_path() -> PathBuf {
    notes_path_from(
        std::env::var_os("GIM_NOTES"),
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("HOME"),
    )
}

pub fn notes_path_from(
    gim_notes: Option<OsString>,
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
) -> PathBuf {
    let non_empty = |v: Option<OsString>| v.filter(|s| !s.is_empty());
    if let Some(p) = non_empty(gim_notes) {
        return PathBuf::from(p);
    }
    if let Some(xdg) = non_empty(xdg_data_home) {
        return PathBuf::from(xdg).join("gim").join("notes.md");
    }
    let home = non_empty(home).unwrap_or_else(|| OsString::from("."));
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("gim")
        .join("notes.md")
}

/// A fresh empty directory under the system temp dir, unique per call.
#[cfg(test)]
pub(crate) fn temp_dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let name = format!("gim-test-{}-{n}", std::process::id());
    let dir = std::env::temp_dir().join(name);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> (Document, String, Vec<u8>) {
        let dir = temp_dir();
        let path = dir.join("note.txt");
        fs::write(&path, bytes).unwrap();
        let (mut doc, text) = load(&path).unwrap();
        let outcome = save(&mut doc, &text).unwrap();
        assert!(outcome.atomic);
        let written = fs::read(&path).unwrap();
        assert_eq!(outcome.bytes, written.len());
        let _ = fs::remove_dir_all(&dir);
        (doc, text, written)
    }

    #[test]
    fn lf_crlf_bom_and_missing_trailing_newline_round_trip() {
        let (doc, text, written) = round_trip(b"a\nb\n");
        assert_eq!(doc.ending, LineEnding::Lf);
        assert_eq!(text, "a\nb\n");
        assert_eq!(written, b"a\nb\n");

        let (doc, text, written) = round_trip(b"a\r\nb\r\n");
        assert_eq!(doc.ending, LineEnding::Crlf);
        assert_eq!(text, "a\nb\n");
        assert_eq!(written, b"a\r\nb\r\n");

        let (doc, text, written) = round_trip("\u{feff}x\ty".as_bytes());
        assert!(doc.bom);
        assert_eq!(text, "x\ty");
        assert_eq!(written, "\u{feff}x\ty".as_bytes());

        let (_, text, written) = round_trip(b"no newline");
        assert_eq!(text, "no newline");
        assert_eq!(written, b"no newline");
    }

    #[test]
    fn missing_file_is_new_and_created_on_save_with_parents() {
        let dir = temp_dir();
        let path = dir.join("deep").join("er").join("new.md");
        let (mut doc, text) = load(&path).unwrap();
        assert!(!doc.exists);
        assert_eq!(text, "");
        assert_eq!(doc.display_name, "new.md");
        assert!(doc.is_dirty("x"));
        assert!(!doc.is_dirty(""));
        save(&mut doc, "hello").unwrap();
        assert!(doc.exists);
        assert!(!doc.is_dirty("hello"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_errors() {
        let dir = temp_dir();
        assert!(matches!(load(&dir), Err(LoadError::NotFile(_))));
        let bad = dir.join("bad.txt");
        fs::write(&bad, [0xff, 0xfe, b'a']).unwrap();
        assert!(matches!(load(&bad), Err(LoadError::NotUtf8(_))));
        let msg = load(&bad).unwrap_err().to_string();
        assert!(msg.ends_with("not valid UTF-8"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_target_is_refused_and_permissions_survive_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let path = dir.join("ro.txt");
        fs::write(&path, "keep").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let (mut doc, _) = load(&path).unwrap();
        let err = save(&mut doc, "changed").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(err.to_string(), "file is read-only (chmod +w to save)");
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        save(&mut doc, "changed").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert_eq!(fs::read_to_string(&path).unwrap(), "changed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_writes_the_real_file() {
        let dir = temp_dir();
        let real = dir.join("real.md");
        let link = dir.join("link.md");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let (mut doc, _) = load(&link).unwrap();
        save(&mut doc, "new").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
        assert_eq!(fs::read_to_string(&link).unwrap(), "new");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_path_resolution() {
        let some = |s: &str| Some(OsString::from(s));
        assert_eq!(
            notes_path_from(some("/n/x.md"), some("/xdg"), some("/home")),
            PathBuf::from("/n/x.md")
        );
        assert_eq!(
            notes_path_from(some(""), some("/xdg"), some("/home")),
            PathBuf::from("/xdg/gim/notes.md")
        );
        assert_eq!(
            notes_path_from(None, None, some("/home")),
            PathBuf::from("/home/.local/share/gim/notes.md")
        );
        assert_eq!(
            notes_path_from(None, None, None),
            PathBuf::from("./.local/share/gim/notes.md")
        );
    }
}
