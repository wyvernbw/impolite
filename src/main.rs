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
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::BufReader;
use tokio::io::BufWriter;
use tokio::net::unix;
use tokio::pin;
use tokio::select;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use ratatui::crossterm::event;

use mana_tui::mana_tui_potion;
use mana_tui::prelude::*;

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
}

#[derive(Debug, Clone)]
enum Focus {
    UsernameField,
    PasswordField,
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
        },
        Effect::new(move |tx| {
            let req_rx = req_rx.clone();
            async move {
                if let Err(err) = greetd_task(cli_args, req_rx).await {
                    tx.send(Msg::Error(Arc::new(err)))
                        .wrap_err("Fatal channel error")
                        .unwrap();
                }
            }
        }),
    )
}

async fn greetd_task(cli_args: &'static CliArgs, req_rx: Receiver<greetd::Request>) -> Result<()> {
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
            Ok(res) = greetd_decode(&mut stream) => {}
        }
    }
    Ok(())
}

async fn view(model: &Model) -> View {
    let hostname = hostname();
    let hostname = hostname
        .as_ref()
        .map(|str| str.to_string_lossy())
        .unwrap_or_else(|_| Cow::Borrowed("machine"));
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
                            key!(Tab) => Some((Msg::FocusOn(Focus::PasswordField), Effect::none())),
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
                            key!(Tab) => Some((Msg::FocusOn(Focus::UsernameField), Effect::none())),
                            _ => None
                        }
                    })
                />
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
        true => Style::new().fg(LIPGLOSS[1][0]),
        false => Style::new().fg(LIPGLOSS[5][2]).dim(),
    };
    let input_style = match focused {
        true => Style::new().fg(LIPGLOSS[2][1]),
        false => Style::new(),
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

async fn update(mut model: Model, msg: Msg) -> (Model, Effect<Msg>) {
    match msg {
        Msg::Quit => unreachable!(),
        Msg::Error(report) => {
            panic!("{report:?}")
        }
        Msg::GreetdRes(_res) => (model, Effect::none()),
        Msg::FieldUpdate(field, input) => {
            model.fields[field as usize] = input;
            (model, Effect::none())
        }
        Msg::FocusOn(focus) => (Model { focus, ..model }, Effect::none()),
    }
}
