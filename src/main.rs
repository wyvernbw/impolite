use std::sync::Arc;

use flume::Receiver;
use ratatui::{DefaultTerminal, crossterm::event};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub mod greetd;

pub type Str = Arc<str>;

#[tokio::main(flavor = "local")]
async fn main() -> anyhow::Result<()> {
    app().await?;
    Ok(())
}

enum AppMsg {
    TermEvent(event::Event),
    GreetdEvent(greetd::Response),
}

async fn app() -> anyhow::Result<()> {
    let (event_tx, event_rx) = flume::unbounded::<AppMsg>();
    let event_loop = tokio::task::spawn_local(event_loop());
    let render_loop = tokio::task::spawn_local(render_loop(event_rx));
    tokio::join!(event_loop, render_loop);
    Ok(())
}

async fn event_loop() -> anyhow::Result<()> {
    let mut event_stream = event::EventStream::new();
    let mut greetd_conn = greetd::greetd_connect().await?;
    let codec = LengthDelimitedCodec::new();
    let mut greetd_transport = Framed::new(greetd_conn, codec);

    loop {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                // TODO: event
            }
            Some(Ok(greetd_res)) = greetd::greetd_decode(&mut greetd_transport) => {
                // TODO: greetd res

            }
        }
    }
}

async fn render_loop(event_rx: Receiver<AppMsg>) -> anyhow::Result<()> {
    let mut term = ratatui::init();
    while let Ok(msg) = event_rx.recv_async().await {
        term.draw(|frame| {})?;
    }
    ratatui::restore();
    Ok(())
}
