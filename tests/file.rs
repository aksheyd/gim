mod common;
use common::temp_dir;
use gim::file::{Document, LineEnding, LoadError, create_file, load, notes_path_from, save};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;

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

#[test]
fn create_file_creates_parents_and_empty_file() {
    let dir = temp_dir();
    let path = dir.join("nested").join("folder").join("created.md");
    assert!(!path.exists());
    create_file(&path).unwrap();
    assert!(path.is_file());
    assert_eq!(fs::read(&path).unwrap(), b"");
    let _ = fs::remove_dir_all(&dir);
}
