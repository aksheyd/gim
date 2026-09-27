mod common;
use common::temp_dir;
use std::time::Instant;

use gim::app::App;
use gim::clipboard::MemClipboard;
use gim::file::Document;
use gim::protocol::ServerMsg;
use gim::server::Server;
use gim::{Event, KeyCode, KeyEvent, KeyModifiers};

fn make_server(text: &str) -> Server {
    let doc = Document::new(&temp_dir().join("notes.md"));
    let clipboard = Box::new(MemClipboard::default());
    Server::new(App::new(doc, text.to_string(), clipboard, 80, 24))
}

fn plain(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[test]
fn two_clients_get_the_same_patch() {
    let mut s = make_server("hi");
    let t = Instant::now();
    let a = s.attach(1, 40, 10, t);
    let b = s.attach(2, 40, 10, t);
    assert_eq!(s.clients(), 2);
    assert!(matches!(a[0].1, ServerMsg::Patch(_)));
    assert!(matches!(b[0].1, ServerMsg::Patch(_)));
    let out = s.event(1, plain('x'), t);
    let patches: Vec<_> = out
        .iter()
        .filter_map(|(_, m)| match m {
            ServerMsg::Patch(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0], patches[1]);
}
