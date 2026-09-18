use std::io::{self, Read, Write};
use std::time::Duration;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 16 << 20;
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
pub const PROTOCOL_ERROR: &str = "protocol error";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ClientMsg {
    Hello {
        version: u32,
        width: u16,
        height: u16,
    },
    Event(crossterm::event::Event),
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ServerMsg {
    Patch(Patch),
    Bye,
    Error(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CellUpdate(pub u16, pub u16, pub String, pub bool);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub width: u16,
    pub height: u16,
    pub full: bool,
    pub cells: Vec<CellUpdate>,
    pub cursor: Option<(u16, u16)>,
}

pub fn write_msg<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(&body);
    w.write_all(&framed)
}

pub fn read_msg<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    let mut header = [0u8; 4];
    match r.read_exact(&mut header[..1]) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    r.read_exact(&mut header[1..])?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    match serde_json::from_slice(&body) {
        Ok(msg) => Ok(Some(msg)),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    }
}

fn key(cell: &Cell) -> (&str, bool) {
    (cell.symbol(), cell.modifier.contains(Modifier::REVERSED))
}

pub fn diff(
    prev: Option<&Buffer>,
    prev_cursor: Option<(u16, u16)>,
    next: &Buffer,
    cursor: Option<(u16, u16)>,
) -> Option<Patch> {
    let area = next.area;
    let prev = prev.filter(|p| p.area == area);
    let mut cells = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let now = next.cell((x, y)).map_or((" ", false), key);
            let changed = match prev {
                Some(p) => p.cell((x, y)).map_or((" ", false), key) != now,
                None => now != (" ", false),
            };
            if changed {
                cells.push(CellUpdate(x, y, now.0.to_string(), now.1));
            }
        }
    }
    if prev.is_some() && cells.is_empty() && prev_cursor == cursor {
        return None;
    }
    Some(Patch {
        width: area.width,
        height: area.height,
        full: prev.is_none(),
        cells,
        cursor,
    })
}

pub fn apply(patch: &Patch, mirror: &mut Buffer) {
    if patch.full {
        *mirror = Buffer::empty(Rect::new(0, 0, patch.width, patch.height));
    }
    for CellUpdate(x, y, symbol, reversed) in &patch.cells {
        let Some(cell) = mirror.cell_mut((*x, *y)) else {
            continue;
        };
        cell.reset();
        cell.set_symbol(symbol);
        cell.modifier = if *reversed {
            Modifier::REVERSED
        } else {
            Modifier::empty()
        };
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton,
        MouseEvent, MouseEventKind,
    };

    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(msg: &T) {
        let mut bytes = Vec::new();
        write_msg(&mut bytes, msg).unwrap();
        let mut cursor = Cursor::new(bytes);
        let back: T = read_msg(&mut cursor).unwrap().unwrap();
        assert_eq!(&back, msg);
        assert_eq!(read_msg::<_, T>(&mut cursor).unwrap(), None);
    }

    #[test]
    fn every_variant_round_trips() {
        round_trip(&ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            width: 80,
            height: 24,
        });
        round_trip(&ClientMsg::Event(Event::Paste("a\r\nb\rc".to_string())));
        let key = KeyEvent {
            code: KeyCode::Char('Z'),
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            kind: KeyEventKind::Repeat,
            state: KeyEventState::CAPS_LOCK,
        };
        round_trip(&ClientMsg::Event(Event::Key(key)));
        let mouse = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 3,
            row: 4,
            modifiers: KeyModifiers::ALT,
        };
        round_trip(&ClientMsg::Event(Event::Mouse(mouse)));
        round_trip(&ClientMsg::Event(Event::Resize(1, 1)));
        round_trip(&ClientMsg::Event(Event::FocusGained));
        round_trip(&ClientMsg::Shutdown);
        round_trip(&ServerMsg::Patch(Patch {
            width: 2,
            height: 1,
            full: true,
            cells: vec![CellUpdate(0, 0, "コ".to_string(), true)],
            cursor: Some((1, 0)),
        }));
        round_trip(&ServerMsg::Bye);
        round_trip(&ServerMsg::Error("nope".to_string()));
    }

    #[test]
    fn framing_errors_and_back_to_back_messages() {
        let mut bytes = Vec::new();
        write_msg(&mut bytes, &ServerMsg::Bye).unwrap();
        write_msg(&mut bytes, &ServerMsg::Error("x".to_string())).unwrap();
        let mut cursor = Cursor::new(bytes.clone());
        assert_eq!(read_msg(&mut cursor).unwrap(), Some(ServerMsg::Bye));
        let second: ServerMsg = read_msg(&mut cursor).unwrap().unwrap();
        assert_eq!(second, ServerMsg::Error("x".to_string()));
        assert_eq!(read_msg::<_, ServerMsg>(&mut cursor).unwrap(), None);

        let mut truncated_header = Cursor::new(bytes[..2].to_vec());
        let err = read_msg::<_, ServerMsg>(&mut truncated_header).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        let mut truncated_body = Cursor::new(bytes[..6].to_vec());
        let err = read_msg::<_, ServerMsg>(&mut truncated_body).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);

        let mut oversize = Cursor::new(((MAX_MESSAGE + 1) as u32).to_be_bytes().to_vec());
        let err = read_msg::<_, ServerMsg>(&mut oversize).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let mut garbage = Vec::from(4u32.to_be_bytes());
        garbage.extend_from_slice(b"}}}}");
        let err = read_msg::<_, ServerMsg>(&mut Cursor::new(garbage)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let huge = ClientMsg::Event(Event::Paste("x".repeat(MAX_MESSAGE)));
        let mut sink = Vec::new();
        let err = write_msg(&mut sink, &huge).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(sink.is_empty());
    }
}
