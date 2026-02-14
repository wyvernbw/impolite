use std::net::hostname;

use ratatui::crossterm::event::{Event, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::macros::ratatui_core;
use ratatui::prelude::*;
use ratatui::style::Styled;
use ratatui::widgets::{Block, Padding, Paragraph};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use crate::lipgloss_colors::PALETTE;
use crate::{AppArgs, AppMsg, Str};

pub trait Component {
    type State;
    type UpdateRet = ();
    type RenderRet = ();

    fn update(&self, event: AppMsg, state: &mut Self::State) -> Self::UpdateRet;

    fn render(&self, area: Rect, frame: &mut Frame, state: &mut Self::State) -> Self::RenderRet;
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

#[derive_const(Default)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    #[default]
    UsernameField,
    PasswordField,
}

impl Field {
    fn handle_event(self, event: &Event) -> Self {
        match (&self, event) {
            (
                Self::UsernameField,
                key!(Down) | key!(Char('j'), KeyModifiers::CONTROL) | key!(Tab) | key!(Enter),
            ) => Field::PasswordField,
            (
                Self::PasswordField,
                key!(Up) | key!(Char('k'), KeyModifiers::CONTROL) | key!(Tab),
            ) => Field::UsernameField,
            _ => self,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Field::UsernameField => " Username ",
            Field::PasswordField => " Password ",
        }
    }
}

pub struct Impolite(&'static AppArgs);
pub struct ImpoliteState {
    pub exit_flag: bool,
    pub hostname: Str,
    focus: Field,
    prompts: PromptState,
}

#[derive(Default)]
struct PromptState {
    username: InputComponentState,
    password: InputComponentState,
}

impl Impolite {
    pub const fn new(args: &'static AppArgs) -> Self {
        Self(args)
    }
}

impl ImpoliteState {
    pub fn new() -> Self {
        Self {
            exit_flag: false,
            hostname: hostname()
                .map(|string| string.display().to_string().into())
                .unwrap_or("machine".into()),
            focus: Field::default(),
            prompts: PromptState::default(),
        }
    }

    fn current_prompt_mut(&mut self) -> &mut InputComponentState {
        match self.focus {
            Field::UsernameField => &mut self.prompts.username,
            Field::PasswordField => &mut self.prompts.password,
        }
    }

    fn current_prompt(&self) -> &InputComponentState {
        match self.focus {
            Field::UsernameField => &self.prompts.username,
            Field::PasswordField => &self.prompts.password,
        }
    }

    fn current_prompt_cursor(&self) -> (u16, u16) {
        let current = self.current_prompt();
        let pos = current.position;
        (pos.0 + current.text.visual_cursor() as u16, pos.1)
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
        InputComponent {
            field: Field::UsernameField,
            current_focus: &state.focus,
        }
        .update(event.clone(), &mut state.prompts.username);
        InputComponent {
            field: Field::PasswordField,
            current_focus: &state.focus,
        }
        .update(event.clone(), &mut state.prompts.password);

        match &event {
            AppMsg::TermEvent(key!(Char('c' | 'C'), KeyModifiers::CONTROL)) => {
                state.exit_flag = true;
            }

            AppMsg::TermEvent(event) => {
                state.focus = state.focus.handle_event(event);
            }

            AppMsg::GreetdEvent(_) => {}
            _ => {}
        };
    }

    fn render(&self, area: Rect, frame: &mut Frame<'_>, state: &mut Self::State) {
        let area = Block::new().padding(Padding::uniform(1)).inner(area);
        let [_, area, _] = Layout::horizontal([Constraint::Ratio(1, 3)].repeat(3))
            .flex(layout::Flex::Center)
            .areas(area);
        let [_, area, _] = Layout::vertical([Constraint::Ratio(1, 3)].repeat(3))
            .flex(layout::Flex::Center)
            .areas(area);

        let [heading, separator, area] =
            Layout::vertical([Constraint::Max(1), Constraint::Max(1), Constraint::Fill(1)])
                .spacing(1)
                .areas(area);
        Line::from_iter([
            Span::raw("• Logging into "),
            Span::raw(state.hostname.as_ref())
                .style(Style::new().fg(PALETTE[7][10]))
                .bold(),
        ])
        .render(heading, frame.buffer_mut());

        "─"
            .repeat(separator.width as usize)
            .set_style(Style::new().fg(Color::from_u32(0x004e4e4e)))
            .render(separator, frame.buffer_mut());

        let [user_area, pass_area, rest] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .spacing(1)
        .areas(area);

        InputComponent {
            current_focus: &state.focus,
            field: Field::UsernameField,
        }
        .render(user_area, frame, &mut state.prompts.username);

        InputComponent {
            current_focus: &state.focus,
            field: Field::PasswordField,
        }
        .render(pass_area, frame, &mut state.prompts.password);

        let [help_area] = Layout::vertical([Constraint::Max(3)])
            .flex(layout::Flex::End)
            .areas(rest);

        HelpArea.render(help_area, frame, &mut ());

        frame.set_cursor_position(state.current_prompt_cursor());
    }
}

struct InputComponent<'a> {
    field: Field,
    current_focus: &'a Field,
}

#[derive(Default, Clone)]
struct InputComponentState {
    position: (u16, u16),
    text: Input,
}

impl<'a> Component for InputComponent<'a> {
    type State = InputComponentState;

    fn update(&self, event: AppMsg, state: &mut Self::State) {
        let AppMsg::TermEvent(event) = event else {
            return;
        };

        if self.current_focus == &self.field {
            state.text.handle_event(&event);
        }
    }

    fn render(&self, area: Rect, frame: &mut Frame, state: &mut Self::State) {
        let [label_area, input_area] = Layout::horizontal([
            Constraint::Max(self.field.label().len() as _),
            Constraint::Min(2),
        ])
        .spacing(2)
        .areas(area);

        state.position = (input_area.x, input_area.y);

        let style = match &self.field == self.current_focus {
            true => Style::new()
                .bg(PALETTE[0][0])
                .bold()
                .fg(Color::from_u32(0x00ffffff)),
            false => Style::new()
                .dim()
                .bg(PALETTE[5][2])
                .fg(Color::from_u32(0x00ffffff)),
        };
        self.field
            .label()
            .set_style(style)
            .render(label_area, frame.buffer_mut());
        state.text.value().render(input_area, frame.buffer_mut());
    }
}

struct HelpArea;

impl Component for HelpArea {
    type State = ();

    fn update(&self, _: AppMsg, _: &mut Self::State) {}

    fn render(&self, area: Rect, frame: &mut Frame, _: &mut Self::State) {
        let bind = |text: &'static str| text.fg(Color::from_u32(0x00626262));
        let tooltip = |text: &'static str| text.fg(Color::from_u32(0x004e4e4e));

        Paragraph::new(Text::from_iter([
            Line::from(r#"Impolite login manager • #@!$ you!"#)
                .style(Style::new().fg(Color::from_u32(0x004E4E4E))),
            Line::from(""),
            Line::from(vec![
                bind("^J/K"),
                tooltip(" or "),
                bind("↑↓"),
                tooltip(" or "),
                bind("TAB"),
                tooltip(" navigate • "),
                bind("Enter "),
                tooltip("confirm"),
            ]),
        ]))
        .render(area, frame.buffer_mut());
    }
}
