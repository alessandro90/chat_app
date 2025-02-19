use std::cell::RefCell;
use std::env::{self};
use std::error::Error;
use std::io::ErrorKind;
use std::rc::Rc;
use std::time::{Duration, Instant};

use async_chat::message::ParsedMsg;
use cursive::event::{Event, EventResult};
use cursive::view::ViewWrapper;
use cursive::views::Dialog;
use cursive::{
    event::Key,
    theme::Theme,
    view::{Nameable, Resizable, ScrollStrategy, Scrollable},
    views::{DummyView, LinearLayout, TextArea, TextView},
};
use cursive::{Cursive, CursiveRunnable, CursiveRunner, View};

use crate::connection::{Connection, Reader, Writer};

const CHAT_NAME: &str = "chat_view";
const INPUT_NAME: &str = "input_view";
const DIALOG_NAME: &str = "conn_err_dialog";
const MAX_DURATION_DISCONNECTED: Duration = Duration::from_secs(5);
const MAX_CHAT_LEN_CHARS: usize = 1_024 * 50;
const INFO_PREFIX: &str = "INFO";

type Runner = CursiveRunner<CursiveRunnable>;

pub fn run() {
    let mut args = env::args().skip(1);
    if args.len() != 2 {
        eprintln!("Provide server ip and port to connect");
        return;
    }
    let ip = args.next().unwrap();
    let port = args.next().as_ref().and_then(|p| p.parse().ok()).unwrap();

    let mut siv = cursive::default();
    siv.set_theme(Theme::terminal_default());
    let mut siv = siv.into_runner();
    siv.add_global_callback(Key::Esc, Cursive::quit);

    let mut app = App::new(&mut siv, ip, port);

    siv.refresh();
    while siv.is_running() {
        siv.step();
        app.run(&mut siv);
    }
}

struct App {
    state: State,
    ip: String,
    port: u16,
    retry_requested: Rc<RefCell<bool>>,
    retries: usize,
    time_since_disconnection: Instant,
}

impl App {
    fn new(siv: &mut Runner, ip: String, port: u16) -> Self {
        match Connection::new(&ip, port) {
            Ok(connection) => {
                let app = Self {
                    state: State::Connected,
                    ip,
                    port,
                    retry_requested: Rc::new(RefCell::new(false)),
                    retries: 0,
                    time_since_disconnection: Instant::now(),
                };
                Self::chat_layer(siv, connection, None, None);
                app
            }
            Err(_) => {
                let mut app = Self {
                    state: State::NotConnected,
                    ip,
                    port,
                    retry_requested: Rc::new(RefCell::new(false)),
                    retries: 1,
                    time_since_disconnection: Instant::now(),
                };
                app.dialog_layer(siv);
                app
            }
        }
    }

    fn run(&mut self, siv: &mut Runner) {
        match self.state {
            State::Connected => {
                if let Some(action) = siv
                    .call_on_name(CHAT_NAME, |chat: &mut Chat| chat.check_messages())
                    .flatten()
                {
                    match action {
                        MessageAction::Refresh => {
                            siv.refresh();
                        }
                        MessageAction::LostConnection => {
                            self.state = State::NotConnected;
                            self.time_since_disconnection = Instant::now();
                            self.dialog_layer(siv);
                            siv.refresh();
                        }
                    };
                }
            }
            State::NotConnected => {
                if !(*self.retry_requested).borrow().to_owned()
                    && self.time_since_disconnection.elapsed() < MAX_DURATION_DISCONNECTED
                {
                    return;
                }
                *self.retry_requested.borrow_mut() = false;
                self.time_since_disconnection = Instant::now();
                match Connection::new(&self.ip, self.port) {
                    Ok(connection) => {
                        self.state = State::Connected;
                        self.retries = 0;

                        let input_text = siv
                            .call_on_name(INPUT_NAME, |input: &mut Input| {
                                input.with_view(|text| text.get_content().to_owned())
                            })
                            .flatten();

                        let chat_text = siv
                            .call_on_name(CHAT_NAME, |chat: &mut Chat| {
                                chat.with_view_mut(|text| text.get_content().source().to_owned())
                            })
                            .flatten();

                        siv.pop_layer();
                        siv.pop_layer();
                        Self::chat_layer(siv, connection, chat_text, input_text);
                    }
                    Err(_) => {
                        self.retries = self.retries.wrapping_add(1);
                        let retries = self.retries;
                        siv.call_on_name(DIALOG_NAME, move |view: &mut Dialog| {
                            view.set_content(TextView::new(unable_to_connect_text(retries)));
                        });
                    }
                };
                siv.refresh();
            }
        };
    }

    fn chat_layer(
        siv: &mut Cursive,
        connection: Connection,
        chat_text: Option<String>,
        input_text: Option<String>,
    ) {
        let (writer, reader) = connection.split();
        let screen = LinearLayout::vertical()
            .child(
                Chat::new(reader, chat_text)
                    .with_name(CHAT_NAME)
                    .full_width()
                    .full_height()
                    .scrollable()
                    .scroll_strategy(ScrollStrategy::StickToBottom),
            )
            .child(DummyView)
            .child(
                Input::new(writer, input_text)
                    .with_name(INPUT_NAME)
                    .full_width()
                    .scrollable()
                    .scroll_strategy(ScrollStrategy::StickToBottom),
            );
        siv.add_fullscreen_layer(screen);
    }

    fn dialog_layer(&mut self, siv: &mut Runner) {
        let retry_requested = Rc::clone(&self.retry_requested);
        siv.add_layer(
            Dialog::text(unable_to_connect_text(self.retries))
                .button("Try again", move |_| {
                    *retry_requested.borrow_mut() = true;
                })
                .button("Quit", |s| Cursive::quit(s))
                .with_name(DIALOG_NAME),
        );
    }
}

fn unable_to_connect_text(retries: usize) -> String {
    format!("Unable to connect to server. Retry no. {}", retries)
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    NotConnected,
    Connected,
}

#[derive(Debug, PartialEq, Eq)]
enum MessageAction {
    Refresh,
    LostConnection,
}

struct Chat {
    reader: Reader,
    text_view: TextView,
}

impl Chat {
    #[must_use]
    fn new(reader: Reader, text: Option<String>) -> Self {
        Self {
            reader,
            text_view: TextView::new(text.unwrap_or("".to_string())),
        }
    }

    fn check_text_len(&mut self) {
        let chars = self.text_view.get_content();
        let chars = chars.source();
        if chars.len() > MAX_CHAT_LEN_CHARS {
            self.text_view
                .set_content(chars[chars.len() / 2..].to_string());
        }
    }

    #[must_use]
    fn check_messages(&mut self) -> Option<MessageAction> {
        if let Some(msg) = self.reader.try_read_msg() {
            match msg {
                Ok(ParsedMsg::Command(_) | ParsedMsg::Info(_)) => {
                    panic!("Invalid message type from server {:#?}", msg)
                }
                Ok(ParsedMsg::UserCount(n)) => {
                    self.text_view.append(format!(
                        "{}.User-Count: {}\n\n",
                        INFO_PREFIX,
                        n.to_string()
                    ));
                    self.check_text_len();
                    Some(MessageAction::Refresh)
                }
                Ok(ParsedMsg::Help(text)) => {
                    self.text_view
                        .append(format!("{}.Help:\n{}\n\n", INFO_PREFIX, text));
                    self.text_view.append("\n\n");
                    self.check_text_len();
                    Some(MessageAction::Refresh)
                }
                Ok(ParsedMsg::Text(text)) => {
                    self.text_view.append(text);
                    self.text_view.append("\n\n");
                    self.check_text_len();
                    Some(MessageAction::Refresh)
                }
                Err(_) => Some(MessageAction::LostConnection),
            }
        } else {
            None
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
    fn new(writer: Writer, text: Option<String>) -> Self {
        let text_area = match text {
            Some(s) => {
                let mut text_area = TextArea::new().content(s.to_owned());
                text_area.set_cursor(s.len());
                text_area
            }
            None => TextArea::new().content(""),
        };
        Self { text_area, writer }
    }
}

impl ViewWrapper for Input {
    type V = TextArea;
    fn wrap_on_event(&mut self, ch: Event) -> EventResult {
        match ch {
            Event::CtrlChar('s') => {
                let content = self.text_area.get_content();
                if let Err(e) = self.writer.try_send_msg(content) {
                    if e.kind() == ErrorKind::Other {
                        let err_string =
                            e.source().map(|e| e.to_string()).unwrap_or("".to_string());
                        self.text_area.set_content(format!("{}\n\n", err_string));
                    }
                } else {
                    self.text_area.set_content("");
                }
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
