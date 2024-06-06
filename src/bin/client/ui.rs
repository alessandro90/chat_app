use std::net::TcpStream;

use cursive::event::Event;
use cursive::{
    event::Key,
    theme::Theme,
    view::{Nameable, Resizable, ScrollStrategy, Scrollable},
    views::{DummyView, EditView, LinearLayout, TextArea, TextView},
};

const CHAT_VIEW_NAME: &str = "chat_view";
const INPUT_NAME: &str = "input_view";

pub fn run() {
    // TODO: to listen to incoming messages we can make a global callback on the refresh event with
    // on_global_callback
    let mut siv = cursive::default();
    siv.set_theme(Theme::terminal_default());
    siv.add_global_callback(Key::Esc, |s| s.quit());
    // siv.set_global_callback(Event::Refresh, |_| {});
    siv.set_on_pre_event(Event::CtrlChar('s'), |s| {
        let text = s.call_on_name(INPUT_NAME, |input: &mut TextArea| {
            input.get_content().to_owned()
        });
        if let Some(text) = text {
            if !text.is_empty() {
                s.call_on_name(CHAT_VIEW_NAME, |view: &mut TextView| {
                    view.append("\n\n");
                    view.append(text);
                });
                s.call_on_name(INPUT_NAME, |input: &mut EditView| input.set_content(""));
            }
        }
        s.call_on_name(INPUT_NAME, |input: &mut TextArea| input.set_content(""));
    });
    let screen = LinearLayout::vertical()
        .child(
            TextView::new("")
                .with_name(CHAT_VIEW_NAME)
                .full_width()
                .full_height()
                .scrollable()
                .scroll_strategy(ScrollStrategy::StickToBottom),
        )
        .child(DummyView)
        .child(
            TextArea::new()
                .content("")
                .with_name(INPUT_NAME)
                .full_width()
                .scrollable()
                .scroll_strategy(ScrollStrategy::StickToBottom),
        );
    siv.add_fullscreen_layer(screen);
    siv.run();
}
