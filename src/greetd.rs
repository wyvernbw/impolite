use std::{net::SocketAddr, sync::Arc};

use anyhow::anyhow;
use facet::Facet;
use freedesktop_desktop_entry::{DesktopEntry, desktop_entries, get_languages_from_env};
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::{
    bytes::Bytes,
    codec::{Framed, LengthDelimitedCodec},
};

use crate::Str;

pub async fn get_sessions() -> Vec<DesktopEntry> {
    let locales = get_languages_from_env();

    desktop_entries(&locales)
}

#[derive(Facet)]
#[repr(u8)]
#[facet(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession {
        username: Str,
    },
    PostAuthMessageResponse,
    StartSession {
        command: Arc<[Str]>,
        env: Arc<[Str]>,
    },
    CancelSession,
}

#[derive(Facet)]
#[repr(u8)]
#[facet(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error {
        error_type: ErrorType,
        description: Str,
    },
    #[facet(transparent)]
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: Str,
    },
}

#[derive(Facet)]
#[repr(u8)]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Facet)]
#[repr(u8)]
pub enum ErrorType {
    AuthError,
    Error,
}

pub fn greetd_socket_addr() -> anyhow::Result<SocketAddr> {
    let addr = std::env::var("GREETD_SOCK")?;
    let addr = addr.parse()?;
    Ok(addr)
}

pub async fn greetd_connect() -> anyhow::Result<tokio::net::TcpStream> {
    let addr = greetd_socket_addr()?;
    let conn = tokio::net::TcpStream::connect(addr).await?;
    Ok(conn)
}

pub async fn greetd_decode(
    transport: &mut Framed<TcpStream, LengthDelimitedCodec>,
) -> Option<anyhow::Result<Response>> {
    let bytes = transport.next().await?;
    match bytes {
        Ok(bytes) => Some(greetd_decode_impl(&bytes)),
        Err(err) => Some(Err(anyhow!(err))),
    }
}

fn greetd_decode_impl(bytes: &[u8]) -> anyhow::Result<Response> {
    let string = std::str::from_utf8(&bytes)?;
    let res = facet_json::from_str(string)?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use crate::greetd::{Request, Response};

    #[test]
    fn serialize_messages() -> anyhow::Result<()> {
        let msg = Request::CreateSession {
            username: "Bingus".into(),
        };

        assert_eq!(
            facet_json::to_string(&msg)?,
            r#"{"type":"create_session","username":"Bingus"}"#
        );

        let msg = Response::Success;

        assert_eq!(facet_json::to_string(&msg)?, r#"{"type":"success"}"#);

        Ok(())
    }
}
