use std::borrow::Cow;
use std::io::BufWriter;
use std::net::hostname;
use std::os::unix::net::UnixStream;

use ratatui::crossterm::event::KeyModifiers;
use ratatui::prelude::*;
use ratatui::style::Styled;
use ratatui::widgets::{Block, Padding, Paragraph};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use crate::greetd::{GreetdWrite, Request, Response};
use crate::lipgloss_colors::PALETTE;

pub trait Component {
    type State;
    type UpdateRet = ();
    type RenderRet = ();

    fn update(&mut self, event: AppMsg, state: &mut Self::State) -> Self::UpdateRet;

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

#[derive(Debug)]
enum FormState {
    WaitingForSession,
    WaitingForSessionSuccess,
    WaitingForLoginSuccess,
    Failed(Str),
    PickingDesktop,
    Done,
}

impl Field {
    fn label(&self, is_focused: Option<bool>) -> &'static str {
        let is_focused = is_focused.unwrap_or_default();
        match (self, is_focused) {
            (Field::UsernameField, false) => "  Username ",
            (Field::PasswordField, false) => "  Password ",
            (Field::UsernameField, true) => "| Username",
            (Field::PasswordField, true) => "| Password",
        }
    }

    fn is(&self, other: Field) -> bool {
        *self == other
    }
}

pub struct Impolite<'a>(&'static AppArgs, Option<&'a mut BufWriter<UnixStream>>);
pub struct ImpoliteState {
    pub render_mode: RenderMode,
    pub exit_flag: bool,
    pub hostname: Str,
    pub error: Option<color_eyre::Report>,
    last_response: Option<Response>,
    focus: Field,
    prompts: PromptState,
    form_state: FormState,
}

#[derive(Default)]
struct PromptState {
    username: InputComponentState,
    password: InputComponentState,
}

impl<'a> Impolite<'a> {
    pub const fn new(
        args: &'static AppArgs,
        socket: Option<&'a mut BufWriter<UnixStream>>,
    ) -> Self {
        Self(args, socket)
    }
}

impl ImpoliteState {
    pub fn new() -> Self {
        let host = hostname()
            .map(|string| string.display().to_string())
            .unwrap_or("machine".into());
        let host = format!(" {host} ");
        Self {
            render_mode: RenderMode::Reactive,
            exit_flag: false,
            hostname: host.into(),
            focus: Field::default(),
            prompts: PromptState::default(),
            form_state: FormState::WaitingForSession,
            last_response: None,
            error: None,
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

impl<'a> Impolite<'a> {
    fn greetd_write(&mut self, state: &mut ImpoliteState, req: Request) {
        let res = self.1.greetd_write(req);
        let err = res.err();
        state.error = err;
    }
}

impl<'a> Component for Impolite<'a> {
    type State = ImpoliteState;

    fn update(&mut self, event: AppMsg, state: &mut Self::State) {
        let input_event =
            UsernameInput::new(&mut state.focus).update(event.clone(), &mut state.prompts.username);
        let input_event = input_event
            .or(PasswordInput::new(&mut state.focus)
                .update(event.clone(), &mut state.prompts.password));
        match input_event {
            Some(FormInputEvent::Confirm) => {
                let res = self.1.greetd_write(Request::CreateSession {
                    username: state.prompts.username.text.value().into(),
                });
                let err = res.err();
                state.form_state = FormState::WaitingForSessionSuccess;
                state.error = err;
            }
            Some(FormInputEvent::FocusOn(field)) => {
                state.focus = field;
            }
            None => {}
        }

        match event {
            AppMsg::TermEvent(key!(Char('c' | 'C'), KeyModifiers::CONTROL)) => {
                state.exit_flag = true;
            }
            AppMsg::GreetdEvent(res) => {
                state.last_response = Some(res.clone());
                match (&state.form_state, res) {
                    (FormState::WaitingForSession, _) => {}
                    (FormState::WaitingForSessionSuccess, Response::Success) => {
                        state.form_state = FormState::WaitingForLoginSuccess;
                        self.greetd_write(
                            state,
                            Request::PostAuthMessageResponse {
                                response: Some(state.prompts.password.text.value().into()),
                            },
                        );
                    }
                    (
                        FormState::WaitingForSessionSuccess,
                        Response::Error {
                            error_type,
                            description,
                        },
                    ) => todo!(),
                    (
                        FormState::WaitingForSessionSuccess,
                        Response::AuthMessage {
                            auth_message_type,
                            auth_message,
                        },
                    ) => todo!(),
                    (FormState::Failed(_), Response::Success) => todo!(),
                    (
                        FormState::Failed(_),
                        Response::Error {
                            error_type,
                            description,
                        },
                    ) => todo!(),
                    (
                        FormState::Failed(_),
                        Response::AuthMessage {
                            auth_message_type,
                            auth_message,
                        },
                    ) => todo!(),
                    (FormState::PickingDesktop, Response::Success) => todo!(),
                    (
                        FormState::PickingDesktop,
                        Response::Error {
                            error_type,
                            description,
                        },
                    ) => todo!(),
                    (
                        FormState::PickingDesktop,
                        Response::AuthMessage {
                            auth_message_type,
                            auth_message,
                        },
                    ) => todo!(),
                    (FormState::Done, Response::Success) => todo!(),
                    (
                        FormState::Done,
                        Response::Error {
                            error_type,
                            description,
                        },
                    ) => todo!(),
                    (
                        FormState::Done,
                        Response::AuthMessage {
                            auth_message_type,
                            auth_message,
                        },
                    ) => todo!(),
                    (FormState::WaitingForLoginSuccess, Response::Success) => {
                        state.form_state = FormState::PickingDesktop;
                    }
                    (
                        FormState::WaitingForLoginSuccess,
                        Response::Error {
                            error_type,
                            description,
                        },
                    ) => todo!(),
                    (
                        FormState::WaitingForLoginSuccess,
                        Response::AuthMessage {
                            auth_message_type,
                            auth_message,
                        },
                    ) => todo!(),
                }
            }
            _ => {}
        };
    }

    fn render(&self, area: Rect, frame: &mut Frame<'_>, state: &mut Self::State) {
        let area = Block::new().padding(Padding::uniform(1)).inner(area);
        let area = area.centered(Constraint::Max(48), Constraint::Max(12));

        let [heading, separator, area] =
            Layout::vertical([Constraint::Max(1), Constraint::Max(1), Constraint::Fill(1)])
                .spacing(1)
                .areas(area);
        Line::from_iter([
            Span::raw("• Logging into "),
            Span::raw(state.hostname.as_ref())
                .style(Style::new().bg(PALETTE[6][10]).fg(Color::from_u32(0)))
                .bold(),
        ])
        .render(heading, frame.buffer_mut());

        "─"
            .repeat(separator.width as usize)
            .set_style(Style::new().fg(Color::from_u32(0x004e4e4e)))
            .render(separator, frame.buffer_mut());

        let [user_area, pass_area, pick_desktop_area, rest] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Max(4),
        ])
        .spacing(1)
        .areas(area);

        UsernameInput::new(&mut state.focus).render(user_area, frame, &mut state.prompts.username);
        PasswordInput::new(&mut state.focus).render(pass_area, frame, &mut state.prompts.password);

        if matches!(state.form_state, FormState::PickingDesktop) {
            DesktopPicker.render(pick_desktop_area, frame, &mut DesktopPickerState);
        }

        let [help_area, debug_area] = Layout::vertical([Constraint::Max(3), Constraint::Min(1)])
            .flex(layout::Flex::End)
            .areas(rest);

        HelpArea.render(help_area, frame, &mut ());

        format!("{:?} - {:?}", state.last_response, state.form_state)
            .render(debug_area, frame.buffer_mut());

        frame.set_cursor_position(state.current_prompt_cursor());
    }
}

struct InputComponent {
    field: Field,
    current_focus: Field,
}

#[derive(Default, Clone)]
struct InputComponentState {
    position: (u16, u16),
    text: Input,
}

impl InputComponent {
    fn value<'s>(&'_ self, state: &'s InputComponentState) -> Cow<'s, str> {
        match self.field {
            Field::UsernameField => state.text.value().into(),
            Field::PasswordField => "*".repeat(state.text.value().len()).into(),
        }
    }
}

impl Component for InputComponent {
    type State = InputComponentState;

    fn update(&mut self, event: AppMsg, state: &mut Self::State) {
        let AppMsg::TermEvent(event) = event else {
            return;
        };

        if self.current_focus == self.field {
            state.text.handle_event(&event);
        }
    }

    fn render(&self, area: Rect, frame: &mut Frame, state: &mut Self::State) {
        let [label_area, input_area] = Layout::horizontal([
            Constraint::Max(self.field.label(None).len() as _),
            Constraint::Min(2),
        ])
        .spacing(2)
        .areas(area);

        state.position = (input_area.x, input_area.y);

        let is_focused = self.field == self.current_focus;

        let label_style = match is_focused {
            true => Style::new().fg(PALETTE[0][0]),
            // .fg(Color::from_u32(0x00ffffff)),
            false => Style::new().fg(PALETTE[4][6]), // .bg(PALETTE[5][2])
        };

        let text_style = match is_focused {
            true => Style::new().fg(PALETTE[1][2]).bold(),
            // .fg(Color::from_u32(0x00ffffff)),
            false => Style::new(), // .bg(PALETTE[5][2])
        };

        self.field
            .label(Some(is_focused))
            .set_style(label_style)
            .render(label_area, frame.buffer_mut());
        self.value(state)
            .set_style(text_style)
            .render(input_area, frame.buffer_mut());
    }
}

struct HelpArea;

impl Component for HelpArea {
    type State = ();

    fn update(&mut self, _: AppMsg, _: &mut Self::State) {}

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

fn color_dim(color: Color, by: f32) -> Color {
    if let Color::Rgb(r, g, b) = color {
        let conv = |c: u8, o: u8| {
            let c = c as f32;
            let c = c * (1.0 - by);
            let c = c as u8;
            u32::from(c) << o
        };
        let value = conv(r, 16) + conv(g, 8) + conv(b, 0);
        return Color::from_u32(value);
    }
    color
}

struct UsernameInput<'a> {
    input: InputComponent,
    focus: &'a mut Field,
}

impl<'a> UsernameInput<'a> {
    fn new(current_focus: &'a mut Field) -> Self {
        Self {
            input: InputComponent {
                field: Field::UsernameField,
                current_focus: *current_focus,
            },
            focus: current_focus,
        }
    }
}

struct PasswordInput<'a> {
    input: InputComponent,
    focus: &'a mut Field,
}

enum FormInputEvent {
    Confirm,
    FocusOn(Field),
}

impl<'a> PasswordInput<'a> {
    fn new(current_focus: &'a mut Field) -> Self {
        Self {
            input: InputComponent {
                field: Field::PasswordField,
                current_focus: *current_focus,
            },
            focus: current_focus,
        }
    }
}

impl<'a> Component for UsernameInput<'a> {
    type State = InputComponentState;
    type UpdateRet = Option<FormInputEvent>;

    fn update(&mut self, event: AppMsg, state: &mut Self::State) -> Self::UpdateRet {
        if !self.focus.is(Field::UsernameField) {
            return None;
        }

        if let AppMsg::TermEvent(
            key!(Enter) | key!(Tab) | key!(Char('j'), KeyModifiers::CONTROL) | key!(Down),
        ) = event
        {
            return Some(FormInputEvent::FocusOn(Field::PasswordField));
        };

        self.input.update(event, state);
        None
    }

    fn render(&self, area: Rect, frame: &mut Frame, state: &mut Self::State) -> Self::RenderRet {
        self.input.render(area, frame, state);
    }
}

impl<'a> Component for PasswordInput<'a> {
    type State = InputComponentState;
    type UpdateRet = Option<FormInputEvent>;

    fn update(&mut self, event: AppMsg, state: &mut Self::State) -> Self::UpdateRet {
        if !self.focus.is(Field::PasswordField) {
            return None;
        }

        if let AppMsg::TermEvent(key!(Tab) | key!(Up) | key!(Char('k'), KeyModifiers::CONTROL)) =
            event
        {
            return Some(FormInputEvent::FocusOn(Field::UsernameField));
        };

        if let AppMsg::TermEvent(key!(Enter)) = event {
            return Some(FormInputEvent::Confirm);
        };

        self.input.update(event, state);

        None
    }

    fn render(&self, area: Rect, frame: &mut Frame, state: &mut Self::State) -> Self::RenderRet {
        self.input.render(area, frame, state);
    }
}

struct DesktopPicker;
struct DesktopPickerState;

impl Component for DesktopPicker {
    type State = DesktopPickerState;

    fn update(&mut self, event: AppMsg, state: &mut Self::State) -> Self::UpdateRet {}

    fn render(&self, area: Rect, frame: &mut Frame, state: &mut Self::State) -> Self::RenderRet {}
}
