#![feature(const_default)]
#![feature(derive_const)]
#![feature(gethostname)]
#![feature(const_trait_impl)]
#![feature(associated_type_defaults)]

use color_eyre::Result;
use color_eyre::eyre::Context;
use flume::Receiver;
use flume::Sender;
use mana_tui::mana_tui_potion::Effect;
use mana_tui::mana_tui_potion::Message;
use mana_tui::mana_tui_potion::focus::handlers::On;
use mana_tui::mana_tui_utils::key;
use ratatui::crossterm::event::KeyModifiers;
use std::sync::Arc;
use tokio::io::BufReader;
use tokio::io::BufWriter;
use tokio::select;

use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

use ratatui::crossterm::event;

use mana_tui::mana_tui_potion;
use mana_tui::prelude::*;

use crate::greetd::GreetdWrite;
use crate::greetd::greetd_connect;
use crate::greetd::greetd_decode;

pub mod greetd;
#[path = "lipgloss-colors.rs"]
pub mod lipgloss_colors;

pub type Str = Arc<str>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let subscriber = tracing_subscriber::Registry::default()
        // any number of other subscriber layers may be added before or
        // after the `ErrorLayer`...
        .with(ErrorLayer::default());

    // set the subscriber as the default for the application
    tracing::subscriber::set_global_default(subscriber)?;

    mana_tui_potion::run()
        .init(init)
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
}

impl Message for Msg {
    type Model = Model;
}

struct Model {
    req_tx: Sender<greetd::Request>,
}

async fn init() -> (Model, Effect<Msg>) {
    let (req_tx, req_rx) = flume::unbounded();
    (
        Model {
            req_tx: req_tx.clone(),
        },
        Effect::new(move |tx| {
            let req_rx = req_rx.clone();
            async move {
                if let Err(err) = greetd_task(req_rx).await {
                    tx.send(Msg::Error(Arc::new(err)))
                        .wrap_err("Fatal channel error")
                        .unwrap();
                }
            }
        }),
    )
}

async fn greetd_task(req_rx: Receiver<greetd::Request>) -> Result<()> {
    let mut greetd = greetd_connect().await?;
    let (read, write) = greetd.into_split();
    let mut greetd_read = BufReader::new(read);
    let mut greetd_write = BufWriter::new(write);

    loop {
        select! {
            Ok(req) = req_rx.recv_async() => {
                greetd_write
                    .greetd_write(req).await
                    .wrap_err("error writing request to greetd socket")?;
            }
            Ok(res) = greetd_decode(&mut greetd_read) => {}
        }
    }
    Ok(())
}

async fn view(model: &Model) -> View {
    ui! {
        <Block
            On::new(|_, event| {
                match event {
                    key!(Char('c'), KeyModifiers::CONTROL) => Some((Msg::Quit, Effect::none())),
                    _ => None
                }
            })
        >
        </Block>
    }
}

async fn update(model: Model, msg: Msg) -> (Model, Effect<Msg>) {
    match msg {
        Msg::Quit => unreachable!(),
        Msg::Error(report) => {
            panic!("{report:?}")
        }
        Msg::GreetdRes(response) => (model, Effect::none()),
    }
}
