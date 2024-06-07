use std::env::{self};
use std::io::ErrorKind;

use async_chat::message::ParsedMsg;
use cursive::event::{Event, EventResult};
use cursive::view::ViewWrapper;
use cursive::{
    event::Key,
    theme::Theme,
    view::{Nameable, Resizable, ScrollStrategy, Scrollable},
    views::{DummyView, LinearLayout, TextArea, TextView},
};
use cursive::{Cursive, View};

use crate::connection::{Connection, Reader, Writer};

const CHAT_NAME: &str = "chat_view";
const INPUT_NAME: &str = "input_view";

pub fn run() {
    let mut siv = cursive::default();
    siv.set_theme(Theme::terminal_default());
    let mut siv = siv.into_runner();
    siv.add_global_callback(Key::Esc, Cursive::quit);

    let mut args = env::args().skip(1);
    assert!(args.len() == 2);
    let ip = args.next().unwrap();
    let port = args.next().as_ref().and_then(|p| p.parse().ok()).unwrap();
    let connection = Connection::new(&ip, port).unwrap();
    let (writer, reader) = connection.split();
    let screen = LinearLayout::vertical()
        .child(
            Chat::new(reader)
                .with_name(CHAT_NAME)
                .full_width()
                .full_height()
                .scrollable()
                .scroll_strategy(ScrollStrategy::StickToBottom),
        )
        .child(DummyView)
        .child(
            Input::new(writer)
                .with_name(INPUT_NAME)
                .full_width()
                .scrollable()
                .scroll_strategy(ScrollStrategy::StickToBottom),
        );
    siv.add_fullscreen_layer(screen);

    siv.refresh();
    while siv.is_running() {
        siv.step();
        let should_refresh = siv
            .call_on_name(CHAT_NAME, |chat: &mut Chat| chat.check_messages())
            .unwrap_or(false);
        if should_refresh {
            siv.refresh();
        }
    }
    siv.run();
}

struct Chat {
    reader: Reader,
    text_view: TextView,
}

impl Chat {
    #[must_use]
    fn new(reader: Reader) -> Self {
        Self {
            reader,
            text_view: TextView::new(""),
        }
    }

    #[must_use]
    fn check_messages(&mut self) -> bool {
        if let Ok(msg) = self.reader.try_read_msg() {
            match msg {
                ParsedMsg::Command(_) | ParsedMsg::Info(_) => false,
                ParsedMsg::Num(n) => {
                    self.text_view.append(n.to_string());
                    self.text_view.append("\n\n");
                    true
                }
                ParsedMsg::Text(text) => {
                    self.text_view.append(text);
                    self.text_view.append("\n\n");
                    true
                }
            }
        } else {
            false
        }
    }
}

impl ViewWrapper for Chat {
    type V = TextView;

    fn with_view_mut<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut Self::V) -> R,
    {
        Some(f(&mut self.text_view))
    }

    fn with_view<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&Self::V) -> R,
    {
        Some(f(&self.text_view))
    }
}

struct Input {
    text_area: TextArea,
    writer: Writer,
}

impl Input {
    #[must_use]
    fn new(writer: Writer) -> Self {
        Self {
            text_area: TextArea::new().content(""),
            writer,
        }
    }
}

impl ViewWrapper for Input {
    type V = TextArea;
    fn wrap_on_event(&mut self, ch: Event) -> EventResult {
        match ch {
            Event::CtrlChar('s') => {
                let content = self.text_area.get_content();
                match self.writer.try_send_msg(content) {
                    Ok(_) => {}
                    Err(e) => {
                        if e.kind() == ErrorKind::Other {
                            println!("Other");
                            // TODO: ask server to give max len?
                            // or just write directly. Need a ref to TextView
                        } else {
                            println!("Error");
                            // TODO: could not send message (probably server disconnected)
                        }
                    }
                };
                self.text_area.set_content("");
                EventResult::Consumed(None)
            }
            e => self.text_area.on_event(e),
        }
    }

    fn with_view_mut<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut Self::V) -> R,
    {
        Some(f(&mut self.text_area))
    }

    fn with_view<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&Self::V) -> R,
    {
        Some(f(&self.text_area))
    }
}
