use ratatui::crossterm::event::KeyModifiers;
use ratatui::prelude::style::palette::tailwind as tw;
use ratatui::prelude::*;
use ratatui::style::Styled;
use ratatui::widgets::Block;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::lipgloss_colors::PALETTE;
use crate::{AppArgs, AppMsg};

pub trait Component {
    type State;
    type UpdateRet = ();
    type RenderRet = ();

    fn update(&self, event: AppMsg, state: &mut Self::State) -> Self::UpdateRet;

    fn render(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) -> Self::RenderRet;
}

macro_rules! key {
    ($code:ident) => {
        ::ratatui::crossterm::event::Event::Key(::ratatui::crossterm::event::KeyEvent {
            code: ::ratatui::crossterm::event::KeyCode::$code,
            ..
        })
    };
    ($code:ident ( $($arg:tt)* )) => {
        ::ratatui::crossterm::event::Event::Key(::ratatui::crossterm::event::KeyEvent {
            code: ::ratatui::crossterm::event::KeyCode::$code($($arg)*),
            ..
        })
    };
    ($code:ident ( $($arg:tt)* ), $mods:pat) => {
        ::ratatui::crossterm::event::Event::Key(::ratatui::crossterm::event::KeyEvent {
            code: ::ratatui::crossterm::event::KeyCode::$code($($arg)*),
            modifiers: $mods,
            ..
        })
    };
}

pub struct Impolite(&'static AppArgs);
pub struct ImpoliteState {
    pub exit_flag: bool,
}

impl Impolite {
    pub const fn new(args: &'static AppArgs) -> Self {
        Self(args)
    }
}

impl ImpoliteState {
    pub const fn new() -> Self {
        Self { exit_flag: false }
    }
}

impl Default for ImpoliteState {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Impolite {
    type State = ImpoliteState;

    fn update(&self, event: AppMsg, state: &mut Self::State) {
        match event {
            AppMsg::TermEvent(key!(Char('c' | 'C'), KeyModifiers::CONTROL)) => {
                state.exit_flag = true;
            }

            AppMsg::GreetdEvent(_) => {}
            _ => {}
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        Block::new()
            .style(Style::new().bg(tw::ZINC.c950))
            .render(area, buf);
        let [_, area, _] = Layout::horizontal([Constraint::Ratio(1, 3)].repeat(3))
            .flex(layout::Flex::Center)
            .areas(area);
        InputComponent.render(area, buf, &mut InputComponentState);
    }
}

struct InputComponent;
struct InputComponentState;

impl Component for InputComponent {
    type State = InputComponentState;

    fn update(&self, event: AppMsg, state: &mut Self::State) {}

    fn render(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let [label_area, input_area] =
            Layout::horizontal([Constraint::Max(" Label ".len() as _), Constraint::Min(2)])
                .spacing(2)
                .areas(area);
        " Label "
            .set_style(
                Style::new()
                    .bg(PALETTE[0][0])
                    .fg(Color::from_u32(0x00ffffff)),
            )
            .render(label_area, buf);
        "im typing here".render(input_area, buf);
    }
}
