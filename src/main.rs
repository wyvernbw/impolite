#![feature(associated_type_defaults)]
#![expect(unexpected_cfgs)]

use std::sync::Arc;

use flume::{Receiver, Sender};
use ratatui::crossterm::event;
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::impolite_app::{Component, Impolite, ImpoliteState};

pub mod greetd;
pub mod impolite_app;
pub mod lipgloss_colors;

pub type Str = Arc<str>;

#[tokio::main(flavor = "local")]
async fn main() -> anyhow::Result<()> {
    app().await?;
    Ok(())
}

pub enum AppMsg {
    TermEvent(event::Event),
    GreetdEvent(greetd::Response),
}

async fn app() -> anyhow::Result<()> {
    let (event_tx, event_rx) = flume::unbounded::<AppMsg>();
    let event_loop = tokio::task::spawn_local(event_loop(event_tx));
    let render_loop = tokio::task::spawn_local(render_loop(event_rx));
    render_loop.await??;
    if event_loop.is_finished() {
        event_loop.await??;
    }
    Ok(())
}

async fn event_loop(event_tx: Sender<AppMsg>) -> anyhow::Result<()> {
    let mut event_stream = event::EventStream::new();
    let greetd_conn = greetd::greetd_connect().await;
    match greetd_conn {
        Ok(greetd_conn) => {
            let codec = LengthDelimitedCodec::new();
            let mut greetd_transport = Framed::new(greetd_conn, codec);

            loop {
                tokio::select! {
                    Some(Ok(event)) = event_stream.next() => {
                        event_tx.send_async(AppMsg::TermEvent(event)).await?;
                    }
                    Some(Ok(greetd_res)) = greetd::greetd_decode(&mut greetd_transport) => {
                        event_tx.send_async(AppMsg::GreetdEvent(greetd_res)).await?;
                    }
                }
            }
        }
        Err(err) => {
            tracing::warn!("error connecting to greetd: {err} - running without connection");
            loop {
                tokio::select! {
                    Some(Ok(event)) = event_stream.next() => {
                        event_tx.send_async(AppMsg::TermEvent(event)).await?;
                    }
                }
            }
        }
    }
}

async fn render_loop(event_rx: Receiver<AppMsg>) -> anyhow::Result<()> {
    let mut term = ratatui::init();

    let app = Impolite::new();
    let mut app_state = ImpoliteState::new();

    term.draw(|frame| {
        app.render(frame.area(), frame.buffer_mut(), &mut app_state);
    })?;

    while let Ok(msg) = event_rx.recv_async().await {
        app.update(msg, &mut app_state);
        if app_state.exit_flag {
            break;
        }
        term.draw(|frame| {
            app.render(frame.area(), frame.buffer_mut(), &mut app_state);
        })?;
    }
    ratatui::restore();
    Ok(())
}
