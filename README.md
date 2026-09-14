# gim

A small terminal notes editor with no modes. It follows readline and macOS
text-field conventions: Ctrl-A/E, Option-arrows, Shift-arrows to select,
Ctrl-Z to undo. Soft wrap, mouse selection, one file at a time.

```
gim            # opens your default notes file
gim notes.md   # opens a specific file
```

The default notes file is `$GIM_NOTES` if set, else `$XDG_DATA_HOME/gim/notes.md`,
else `~/.local/share/gim/notes.md`. Missing files (and their directories) are
created on the first save.

## Install

```
cargo install --path .
```

Requires a stable Rust toolchain (1.88 or newer).

## Keys

| Keys | Action |
|---|---|
| Left / Right, Ctrl-B / Ctrl-F | Move by one character |
| Option-Left / Right, Ctrl-Left / Right, Option-b / Option-f | Move by word |
| Ctrl-A / Ctrl-E | Line start / end; again to reach the previous / next line |
| Home / End | Line start / end |
| Up / Down, Ctrl-P / Ctrl-N | Move by visual row, keeping the column |
| PageUp / PageDown | Move by one screen |
| Ctrl-Home / Ctrl-End, Cmd-Up / Cmd-Down | Start / end of the file |
| Shift + any motion | Extend the selection |
| Cmd-A, Option-a | Select all |
| Esc | Clear the selection or the status message |
| Enter | Insert a newline (always) |
| Tab | Insert a tab (shown four columns wide) |
| Backspace / Delete, Ctrl-H / Ctrl-D | Delete one character |
| Option-Backspace, Ctrl-Backspace | Delete the word before the cursor |
| Option-Delete, Option-d | Delete the word after the cursor |
| Ctrl-W | Delete back to the previous space |
| Ctrl-U / Ctrl-K | Delete to the line start / end; at the edge, join the lines |
| Ctrl-Y | Reinsert the last deleted word or line |
| Ctrl-Z, Cmd-Z | Undo |
| Ctrl-Shift-Z, Cmd-Shift-Z, Option-z, Ctrl-R | Redo |
| Ctrl-C, Cmd-C | Copy the selection (without one: does nothing, never quits) |
| Ctrl-X, Cmd-X | Cut the selection |
| Ctrl-V, Cmd-V | Paste |
| Ctrl-S, Cmd-S | Save now (saving is automatic; this just reports it) |
| Ctrl-Q | Save and quit |
| Ctrl-L | Scroll the cursor row to the middle |

Saving is automatic: the file is written one second after you stop typing and
again when you quit, so Ctrl-Q is all you need. The status line shows `[+]`
while a change is not yet on disk. If a write fails, the error appears in the
status line; if that happens on quit, a prompt offers `y` to retry, `n` to
discard the changes, and Esc, Ctrl-Q or Ctrl-C to keep editing.

Word deletes (Option-Backspace, Option-d, Ctrl-W, Ctrl-U, Ctrl-K) go to a
single kill slot that Ctrl-Y reinserts; consecutive kills replace it rather
than accumulate. The clipboard (Ctrl-C/X/V) is separate.

Mouse: click to place the cursor, drag to select (the selection is copied on
release), double-click a word, triple-click a line, wheel to scroll without
moving the cursor.

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
- One undo history per run. If the process is killed, at most the last second
  of typing is lost.
- No file locking: two gim windows on the same file overwrite each other.
- Mixed line endings: the first ending found decides how the file is saved;
  stray carriage returns in an LF file are shown as `?` and kept.
