#![feature(const_default)]
#![feature(derive_const)]
#![feature(gethostname)]
#![feature(const_trait_impl)]
#![feature(associated_type_defaults)]

use color_eyre::Result;
use color_eyre::eyre::Context;
use color_eyre::eyre::eyre;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::IsTerminal;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::{Arc, mpsc};
use std::time::Duration;
use tracing::instrument;

use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use ratatui::crossterm::event;

use crate::impolite::{Component, Impolite, ImpoliteState};

pub mod greetd;
#[path = "impolite/impolite.rs"]
pub mod impolite;
#[path = "lipgloss-colors.rs"]
pub mod lipgloss_colors;

pub type Str = Arc<str>;

fn main() -> Result<()> {
    dbg!(std::io::stdout().is_terminal());
    let subscriber = tracing_subscriber::Registry::default()
        // any number of other subscriber layers may be added before or
        // after the `ErrorLayer`...
        .with(ErrorLayer::default());

    // set the subscriber as the default for the application
    tracing::subscriber::set_global_default(subscriber)?;

    color_eyre::install()?;

    app()
}

#[derive(Debug)]
pub struct AppArgs {
    debug: bool,
}

impl AppArgs {
    fn try_new() -> Result<Self> {
        let mut pargs = pico_args::Arguments::from_env();
        Ok(Self {
            debug: pargs.contains(["-d", "--debug"]),
        })
    }
}

#[derive(Debug, Clone)]
pub enum AppMsg {
    TermEvent(event::Event),
    GreetdEvent(greetd::Response),
}

fn app() -> Result<()> {
    let args = AppArgs::try_new().wrap_err("failed to parse cli arguments")?;
    let args: &'static AppArgs = Box::leak(Box::new(args));

    let (event_tx, event_rx) = mpsc::channel::<AppMsg>();
    let socket = try_connect(args)?;
    let socket_2 = match socket {
        Some(ref socket) => Some(BufWriter::new(
            socket
                .try_clone()
                .wrap_err("failed to clone greetd connection")?,
        )),
        None => None,
    };
    let event_thread = std::thread::spawn(move || event_loop(args, event_tx, socket));
    let render_thread = std::thread::spawn(move || render_loop(args, event_rx, socket_2));

    render_thread
        .join()
        .map_err(|_| eyre!("thread join error"))??;

    if event_thread.is_finished() {
        event_thread
            .join()
            .map_err(|_| eyre!("thread join error"))??;
    }

    Ok(())
}

fn try_connect(args: &AppArgs) -> Result<Option<UnixStream>> {
    let socket = greetd::greetd_connect();

    let socket = match (socket, args.debug) {
        (Ok(socket), _) => Some(socket),
        (Err(_), true) => None,
        (Err(report), false) => return Err(report),
    };
    Ok(socket)
}

#[instrument(ret, err, skip(event_tx))]
fn event_loop(
    args: &'static AppArgs,
    event_tx: Sender<AppMsg>,
    socket: Option<UnixStream>,
) -> Result<()> {
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while let Ok(event) = ratatui::crossterm::event::read() {
                if event_tx.send(AppMsg::TermEvent(event)).is_err() {
                    break;
                }
            }
        });

        if let Some(socket) = socket {
            scope.spawn(|| {
                let mut transport = BufReader::new(socket);
                while let Ok(res) = greetd::greetd_decode(&mut transport) {
                    if event_tx.send(AppMsg::GreetdEvent(res)).is_err() {
                        break;
                    }
                }
            });
        };
    });
    Ok(())
}

pub enum RenderMode {
    Reactive,
    Loop,
}

#[instrument(skip(event_rx, socket), err)]
fn render_loop(
    args: &'static AppArgs,
    event_rx: Receiver<AppMsg>,
    mut socket: Option<BufWriter<UnixStream>>,
) -> color_eyre::Result<()> {
    let mut term = ratatui::init();

    let mut app = Impolite::new(args, socket.as_mut());
    let mut app_state = ImpoliteState::new();

    term.draw(|frame| {
        app.render(frame.area(), frame, &mut app_state);
    })?;

    loop {
        match app_state.render_mode {
            RenderMode::Reactive => {
                let Ok(msg) = event_rx.recv() else {
                    break;
                };
                app.update(msg, &mut app_state);

                if app_state.exit_flag {
                    break;
                }
                term.draw(|frame| {
                    app.render(frame.area(), frame, &mut app_state);
                })?;
            }
            RenderMode::Loop => {
                if let Ok(msg) = event_rx.try_recv() {
                    app.update(msg, &mut app_state);
                    if app_state.exit_flag {
                        break;
                    }
                    term.draw(|frame| {
                        app.render(frame.area(), frame, &mut app_state);
                    })?;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    ratatui::restore();
    Ok(())
}
