use std::{path::PathBuf, sync::Arc};

use color_eyre::{Result, Section, eyre::Context};
use freedesktop_desktop_entry::{DesktopEntry, desktop_entries, get_languages_from_env};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::BufWriter;
use tokio::net::UnixStream;

use tracing::instrument;

use crate::Str;

pub fn get_desktops() -> Vec<DesktopEntry> {
    let locales = get_languages_from_env();

    desktop_entries(&locales)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession {
        username: Str,
    },
    PostAuthMessageResponse {
        response: Option<Str>,
    },
    StartSession {
        command: Arc<[Str]>,
        env: Arc<[Str]>,
    },
    CancelSession,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error {
        error_type: ErrorType,
        description: Str,
    },
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: Str,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum ErrorType {
    AuthError,
    Error,
}

#[instrument(err)]
pub fn greetd_socket_addr() -> Result<PathBuf> {
    let path = std::env::var("GREETD_SOCK")
        .wrap_err("failed to read GREETD_SOCK from env")
        .suggestion(
            "Greetd must be running for Impolite to work. You might already be logged in.",
        )?;
    let path = path.parse()?;
    Ok(path)
}

#[instrument(err)]
pub async fn greetd_connect() -> Result<UnixStream> {
    let addr = greetd_socket_addr()?;
    let conn = UnixStream::connect(addr).await?;
    tracing::info!("CONNECTED ON {conn:?}");
    Ok(conn)
}

#[instrument(skip_all, err)]
pub async fn greetd_decode<A: AsyncRead + Unpin>(transport: &mut A) -> Result<Response> {
    let mut len_buf = [0u8; 4];
    transport.read_exact(&mut len_buf).await?;
    let len = u32::from_ne_bytes(len_buf);
    tracing::info!("RECV {len} bytes");
    let mut buf = vec![0u8; len as _];
    transport.read_exact(&mut buf).await?;
    greetd_decode_impl(&buf)
}

#[instrument(err)]
fn greetd_decode_impl(bytes: &[u8]) -> Result<Response> {
    let string = std::str::from_utf8(bytes)?;
    tracing::info!("GOT {string}");
    let res = serde_json::from_str(string)?;
    Ok(res)
}

pub(crate) trait GreetdWrite {
    async fn greetd_write(&mut self, msg: Request) -> Result<()>;
}

impl<W> GreetdWrite for W
where
    W: AsyncWrite + Unpin,
{
    #[instrument(skip_all, err)]
    async fn greetd_write(&mut self, msg: Request) -> Result<()> {
        let msg = serde_json::to_string(&msg).wrap_err("failed to serialize msg")?;
        {
            let msg = msg.as_bytes();
            let len = msg.len();
            self.write_all(&u32::to_ne_bytes(len as u32))
                .await
                .wrap_err("failed to write length prefix over greetd socket")?;
            self.write_all(msg)
                .await
                .wrap_err("failed to write over greetd socket")?;
        }
        self.flush()
            .await
            .wrap_err("failed to flush greetd socket")?;
        tracing::info!("WROTE {msg}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::greetd::{Request, Response};

    #[test]
    fn serialize_create_session() -> color_eyre::Result<()> {
        let msg = Request::CreateSession {
            username: "Bingus".into(),
        };

        assert_eq!(
            serde_json::to_string(&msg)?,
            r#"{"type":"create_session","username":"Bingus"}"#
        );

        Ok(())
    }

    #[test]
    fn serialize_success() -> color_eyre::Result<()> {
        let msg = Response::Success;

        assert_eq!(serde_json::to_string(&msg)?, r#"{"type":"success"}"#);

        Ok(())
    }

    #[test]
    fn serialize_post_auth_message_response() -> color_eyre::Result<()> {
        let msg = Request::PostAuthMessageResponse {
            response: Some("1234".into()),
        };

        assert_eq!(
            serde_json::to_string(&msg)?,
            r#"{"type":"post_auth_message_response","response":"1234"}"#
        );

        Ok(())
    }
}
