#![feature(const_default)]
#![feature(derive_const)]
#![feature(gethostname)]
#![feature(const_trait_impl)]
#![feature(associated_type_defaults)]

use clap::Parser;
use color_eyre::Result;
use color_eyre::eyre::Context;
use flume::Receiver;
use flume::Sender;
use freedesktop_desktop_entry::DesktopEntry;
use mana_tui::mana_tui_potion::Effect;
use mana_tui::mana_tui_potion::Message;
use mana_tui::mana_tui_potion::focus::handlers::On;
use mana_tui::mana_tui_utils::key;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::text::Span;
use std::borrow::Cow;
use std::net::hostname;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::io::AsyncRead;
use tokio::io::BufReader;
use tokio::io::BufWriter;
use tokio::net::unix;
use tokio::select;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use ratatui::crossterm::event;

use mana_tui::mana_tui_potion;
use mana_tui::prelude::*;

use crate::greetd::ErrorType;
use crate::greetd::GreetdWrite;
use crate::greetd::greetd_connect;
use crate::greetd::greetd_decode;
use crate::lipgloss_colors::LIPGLOSS;

pub mod greetd;
#[path = "lipgloss-colors.rs"]
pub mod lipgloss_colors;

pub type Str = Arc<str>;

#[derive(clap::Parser)]
struct CliArgs {
    #[arg(short, long)]
    debug: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli_args = Box::leak(Box::new(CliArgs::parse())) as &'static _;
    let subscriber = tracing_subscriber::Registry::default()
        // any number of other subscriber layers may be added before or
        // after the `ErrorLayer`...
        .with(ErrorLayer::default());

    // set the subscriber as the default for the application
    tracing::subscriber::set_global_default(subscriber)?;

    mana_tui_potion::run()
        .init(|| init(cli_args))
        .view(view)
        .quit_signal(|_, msg| matches!(msg, Msg::Quit))
        .update(update)
        .run()
        .await?;

    Ok(())
}

#[derive(Debug, Clone)]
enum Msg {
    Quit,
    Error(Arc<color_eyre::Report>),
    GreetdRes(greetd::Response),
    FieldUpdate(Field, Input),
    FocusOn(Focus),
    SubmitLogin,

    Nothing,
    StartShell,
}

#[derive(Debug, Clone)]
#[repr(usize)]
enum Field {
    Username,
    Password,
}

impl Message for Msg {
    type Model = Model;
}

struct Model {
    cli_args: &'static CliArgs,
    req_tx: Sender<greetd::Request>,
    fields: [tui_input::Input; 2],
    focus: Focus,
    form_state: FormState,
    last_response: Option<greetd::Response>,
    desktops: Vec<DesktopEntry>,
    dekstop_picker_state: Arc<Mutex<ListState>>,
}

impl Model {
    fn field(&self, field: Field) -> &tui_input::Input {
        &self.fields[field as usize]
    }
}

#[derive(Debug, Clone)]
enum FormState {
    Idle,
    CreatedSession,
    LoginFailed(ErrorType, Str),
    PickingDesktop,
}

enum FormEffect {
    None,
    SendPassword,
    FocusDesktopPicker,
}

impl FormState {
    fn update(self, res: greetd::Response) -> (Self, FormEffect) {
        match (self, res) {
            (FormState::Idle, _) => (FormState::Idle, FormEffect::None),
            (FormState::CreatedSession, greetd::Response::Success) => {
                (FormState::PickingDesktop, FormEffect::FocusDesktopPicker)
            }
            (
                FormState::CreatedSession,
                greetd::Response::Error {
                    error_type,
                    description,
                },
            ) => (Self::LoginFailed(error_type, description), FormEffect::None),
            (
                FormState::CreatedSession,
                greetd::Response::AuthMessage {
                    auth_message_type: greetd::AuthMessageType::Secret,
                    auth_message: _,
                },
            ) => (Self::CreatedSession, FormEffect::SendPassword),
            (FormState::CreatedSession, greetd::Response::AuthMessage { .. }) => {
                (Self::CreatedSession, FormEffect::None)
            }
            (FormState::LoginFailed(_, _), greetd::Response::Success) => {
                (FormState::PickingDesktop, FormEffect::None)
            }
            (FormState::LoginFailed(_, _), _) => todo!(),
            (
                _,
                greetd::Response::Error {
                    error_type,
                    description,
                },
            ) => (Self::LoginFailed(error_type, description), FormEffect::None),
            (FormState::PickingDesktop, _) => (FormState::PickingDesktop, FormEffect::None),
        }
    }
}

#[derive(Debug, Clone)]
enum Focus {
    UsernameField,
    PasswordField,
    DesktopPicker,
}

impl Focus {
    /// Returns `true` if the focus is [`UsernameField`].
    ///
    /// [`UsernameField`]: Focus::UsernameField
    #[must_use]
    fn is_username_field(&self) -> bool {
        matches!(self, Self::UsernameField)
    }

    /// Returns `true` if the focus is [`PasswordField`].
    ///
    /// [`PasswordField`]: Focus::PasswordField
    #[must_use]
    fn is_password_field(&self) -> bool {
        matches!(self, Self::PasswordField)
    }
}

async fn init(cli_args: &'static CliArgs) -> (Model, Effect<Msg>) {
    let (req_tx, req_rx) = flume::unbounded();
    (
        Model {
            req_tx: req_tx.clone(),
            cli_args,
            focus: Focus::UsernameField,
            fields: Default::default(),
            form_state: FormState::Idle,
            last_response: None,
            desktops: greetd::get_desktops(),
            dekstop_picker_state: Arc::new(Mutex::new(ListState::default())),
        },
        Effect::new(move |tx| {
            let req_rx = req_rx.clone();
            async move {
                if let Err(err) = greetd_task(cli_args, req_rx, tx.clone()).await {
                    tx.send(Msg::Error(Arc::new(err)))
                        .wrap_err("Fatal channel error")
                        .unwrap();
                }
            }
        }),
    )
}

async fn greetd_task(
    cli_args: &'static CliArgs,
    req_rx: Receiver<greetd::Request>,
    tx: Sender<Msg>,
) -> Result<()> {
    let mut greetd = greetd_connect().await;
    let mut greetd = match (greetd, cli_args.debug) {
        (Ok(greetd), _) => Some(greetd),
        (Err(_), true) => None,
        (Err(err), false) => return Err(err),
    };

    struct GreetdStream(
        Option<(
            BufWriter<unix::OwnedWriteHalf>,
            BufReader<unix::OwnedReadHalf>,
        )>,
    );

    impl AsyncRead for GreetdStream {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            match self.0 {
                Some((_, ref mut read)) => Pin::new(read).poll_read(cx, buf),
                None => std::task::Poll::Pending,
            }
        }
    }

    let mut stream = match greetd {
        Some(greetd) => {
            let (read, write) = greetd.into_split();
            let greetd_read = BufReader::new(read);
            let greetd_write = BufWriter::new(write);
            GreetdStream(Some((greetd_write, greetd_read)))
        }
        None => GreetdStream(None),
    };

    loop {
        select! {
            Ok(req) = req_rx.recv_async() => {
                if let GreetdStream(Some((greetd_write, _))) = &mut stream {
                    greetd_write
                        .greetd_write(req).await
                        .wrap_err("error writing request to greetd socket")?;
                }
            }
            Ok(res) = greetd_decode(&mut stream) => {
                tx.send_async(Msg::GreetdRes(res)).await?;
            }
        }
    }
}

async fn view(model: &Model) -> View {
    let hostname = hostname();
    let hostname = hostname
        .as_ref()
        .map(|str| str.to_string_lossy())
        .unwrap_or_else(|_| Cow::Borrowed("machine"));
    let last_response = &model.last_response;
    let form_state = &model.form_state;

    ui! {
        <Block
            On::new(|_, event| {
                match event {
                    key!(Char('c'), KeyModifiers::CONTROL) => Some((Msg::Quit, Effect::none())),
                    _ => None
                }
            })
            Center
            Width::grow()
            Height::grow()
        >
            <Block Gap(1)>
                <Block Direction::Horizontal>
                    <Span>"Logging into "</Span>
                    <Span .style={Style::new().bg(LIPGLOSS[0][13]).fg(Color::Black)}>" {hostname} "</Span>
                </Block>
                <FieldInput
                    .field={Field::Username}
                    .state={&model.fields[Field::Username as usize]}
                    .label="Username"
                    .focused={model.focus.is_username_field()}
                    On::new(|model: &Model, event| {
                        if !model.focus.is_username_field() {
                            return None;
                        }
                        match event {
                            key!(Tab)
                            | key!(Char('j' | 'J'), KeyModifiers::CONTROL)
                            | key!(Down)
                            | key!(Enter) => Some((Msg::FocusOn(Focus::PasswordField), Effect::none())),
                            _ => None
                        }
                    })
                />
                <FieldInput
                    .field={Field::Password}
                    .state={&model.fields[Field::Password as usize]}
                    .label="Password"
                    .focused={model.focus.is_password_field()}
                    .secret=true
                    On::new(|model: &Model, event| {
                        if !model.focus.is_password_field() {
                            return None;
                        }
                        match event {
                            key!(Enter) => Some((Msg::SubmitLogin, Effect::none())),
                            key!(Tab)
                            | key!(Char('k' | 'K'), KeyModifiers::CONTROL)
                            | key!(Up) => Some((Msg::FocusOn(Focus::UsernameField), Effect::none())),
                            _ => None
                        }
                    })
                />
                <Maybe
                    .cond={matches!(model.form_state, FormState::PickingDesktop)}
                    .then={ui!{
                      <DesktopPicker .model={model}/>
                    }}
                />
                <Span>"{last_response:?}:{form_state:?}"</Span>
                <HelpSection Padding::new(0, 0, 4, 0)/>
            </Block>
        </Block>
    }
}

#[subview]
fn field_input(
    field: Field,
    state: &Input,
    label: &str,
    focused: bool,
    #[builder(default)] secret: bool,
) -> View {
    let value = match secret {
        false => Cow::Borrowed(state.value()),
        true => Cow::Owned("*".repeat(state.value().len())),
    };
    let new_state = state.clone();
    let label_style = match focused {
        true => Style::new().fg(LIPGLOSS[6][11]),
        false => Style::new().dim(),
    };
    let input_style = match focused {
        true => Style::new().bold(),
        false => Style::new().dim().bold(),
    };
    let label = match focused {
        true => format!("| {label}"),
        false => format!("  {label}"),
    };
    ui! {
        <Block
            Direction::Horizontal
        >
            <Span .style={label_style}>"{label} "</Span>
            <Span .style={input_style}
                On::new(move |_, event| -> Option<(Msg, _)> {
                    if !focused {
                        return None;
                    }
                    let mut new_state = new_state.clone();
                    match new_state.handle_event(event) {
                        Some(_) => Some((Msg::FieldUpdate(field.clone(), new_state), Effect::none())),
                        _ => None,
                    }
                })
            >
                "{value}"
            </Span>
        </Block>
    }
}

#[subview]
fn maybe(cond: bool, then: View, r#else: Option<View>) -> View {
    if cond {
        then
    } else {
        r#else.unwrap_or(ui! { "" })
    }
}

#[subview]
fn desktop_picker(model: &Model) -> View {
    let items = model
        .desktops
        .iter()
        .map(|desktop| desktop.path.to_string_lossy().to_string());
    let list_state = model.dekstop_picker_state.clone();
    ui! {
        <Block>
            "Pick a session"
            <List
                .items={items}
                {model.dekstop_picker_state.clone()}
                On::new(move |_, event| match event {
                    key!(Char('j')) | key!(Tab) | key!(Down) => {
                        list_state.lock().unwrap().select_next();
                        None
                    },
                    key!(Char('k')) | key!(Up) => {
                        list_state.lock().unwrap().select_previous();
                        None
                    },
                    key!(Char('b')) => Some((Msg::StartShell, Effect::none())),
                    _ => None
                })
            />
        </Block>
    }
}

#[subview]
fn help_section() -> View {
    let bright = Color::from_u32(0x626262);
    let dark = Color::from_u32(0x4e4e4e);
    ui! {
        <Block Direction::Horizontal>
            <Span .style={Style::new().fg(bright)}>"↑↓ / Tab / ^J ^K "</Span>
            <Span .style={Style::new().fg(dark)}>"navigate • "</Span>
            <Span .style={Style::new().fg(bright)}>"Enter "</Span>
            <Span .style={Style::new().fg(dark)}>"confirm "</Span>
        </Block>
    }
}

async fn update(mut model: Model, msg: Msg) -> (Model, Effect<Msg>) {
    match msg {
        Msg::Quit => unreachable!(),
        Msg::Error(report) => {
            panic!("{report:?}")
        }
        Msg::GreetdRes(res) => {
            let (form_state, form_effect) = model.form_state.clone().update(res.clone());
            match form_effect {
                FormEffect::None => {}
                FormEffect::SendPassword => {
                    model
                        .req_tx
                        .send_async(greetd::Request::PostAuthMessageResponse {
                            response: Some(model.field(Field::Password).value().into()),
                        })
                        .await
                        .unwrap();
                }
                FormEffect::FocusDesktopPicker => model.focus = Focus::DesktopPicker,
            };
            (
                Model {
                    form_state,
                    last_response: Some(res),
                    ..model
                },
                Effect::none(),
            )
        }
        Msg::FieldUpdate(field, input) => {
            model.fields[field as usize] = input;
            (model, Effect::none())
        }
        Msg::FocusOn(focus) => (Model { focus, ..model }, Effect::none()),
        Msg::SubmitLogin => {
            model
                .req_tx
                .send_async(greetd::Request::CreateSession {
                    username: model.field(Field::Username).value().into(),
                })
                .await
                .unwrap();
            let form_state = FormState::CreatedSession;

            (
                Model {
                    form_state,
                    ..model
                },
                Effect::none(),
            )
        }
        Msg::Nothing => (model, Effect::none()),
        Msg::StartShell => {
            println!("DONE");
            model
                .req_tx
                .send_async(greetd::Request::StartSession {
                    cmd: ["/bin/sh".into()].into(),
                    env: [].into(),
                })
                .await
                .unwrap();
            (
                model,
                Effect::new(async |tx| {
                    tx.send_async(Msg::Quit).await.unwrap();
                }),
            )
        }
    }
}
