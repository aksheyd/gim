# Details

## How it runs

Every `gim` window on the same file shares one editor: text, cursor,
selection, undo history, scroll position and status line are identical
everywhere and move in tandem, like two tmux clients on one session. The
first window starts a daemon for the file; the daemon owns the editor and
keeps running after every window closes, so opening `gim` again picks up
exactly where you left off. Ctrl-Q saves and closes only the window you
pressed it in.

- The shared screen is the smallest attached terminal, so a small window
  shrinks the view in the others; bigger windows leave the rest blank.
- `gim --kill [FILE]` saves and stops the daemon; every window exits cleanly.
  If the save fails the daemon refuses and stays up.
- One daemon per real file: relative paths and symlinks to the same file
  share it, and the status line shows the target's name.
- The daemon lives in `~/.local/state/gim/run/` (or `$XDG_STATE_HOME/gim/run/`)
  as `<hash>.sock`, with `<hash>.log` (truncated at each start) and a
  `<hash>.lock` used only while a daemon is being started (never removed,
  harmless). Removing the socket file force-stops the daemon within a second
  (it still tries to save first).
- `gim --local` is the old single-process editor and is refused while a
  daemon holds the file. It is also the fallback when `HOME` is unset or the
  state directory cannot be created.

Missing files are created on the first save; with the daemon the parent
directory must already exist (`--local` creates it).

If a write fails, the error appears in the status line; if that happens on
quit, a prompt offers `y` to retry, `n` to quit without saving (the daemon
keeps the text), and Esc, Ctrl-Q or Ctrl-C to keep editing. The prompt is
shared, so any window may answer it.

Word deletes (Option-Backspace, Option-d, Ctrl-W, Ctrl-U, Ctrl-K) go to a
single kill slot that Ctrl-Y reinserts; consecutive kills replace it rather
than accumulate. The clipboard (Ctrl-C/X/V) is separate.

## Files

- Files are read as UTF-8 and kept as typed; tabs are never expanded.
- CRLF line endings and a leading BOM are detected, shown in the status line,
  and written back on save.
- Saves are atomic (temp file plus rename). A read-only file is refused;
  `chmod +w` it first. If the directory refuses a temp file but the file
  itself is writable, gim writes in place and says so.
- Symlinks are followed, so the real file is replaced and the link survives.
  Hard links are not: the rename leaves the other name pointing at the old
  contents.
- Files over 4 MiB are refused.

## Terminal notes

- macOS: for Option-letter chords (Option-b, Option-f, Option-d, Option-z,
  Option-a) the terminal must send Option as Alt. In Ghostty this is
  `macos-option-as-alt = true`; Option-arrows and Option-Backspace work
  regardless.
- tmux: Ctrl-B is the tmux prefix and never reaches gim; use Left.
- Without the kitty keyboard protocol, Ctrl-Shift-Z is indistinguishable from
  Ctrl-Z; use Option-z or Ctrl-R for redo there. Likewise Ctrl-Backspace
  arrives as plain Backspace and deletes one character; use Option-Backspace.
- With mouse capture on, hold Shift to use the terminal's own selection.

## Limitations

- No search, no line numbers, no syntax highlighting, no configuration file.
- One undo history per daemon. If the daemon is killed, at most the last
  second of typing is lost; the undo history goes with it.
- The daemon never re-reads the file: edits made on disk by another program
  while it runs are overwritten by the next autosave. Stop it with
  `gim --kill` before editing the file elsewhere.
- The daemon has no idle exit; it stays until `--kill` or its socket is removed.
- Mixed line endings: the first ending found decides how the file is saved;
  stray carriage returns in an LF file are shown as `?` and kept.
