use std::{io::Read, os::unix::net::UnixStream, path::PathBuf, sync::Arc};

use color_eyre::{Result, Section, eyre::Context};
use freedesktop_desktop_entry::{DesktopEntry, desktop_entries, get_languages_from_env};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::Str;

pub fn get_sessions() -> Vec<DesktopEntry> {
    let locales = get_languages_from_env();

    desktop_entries(&locales)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
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

#[instrument]
pub fn greetd_socket_addr() -> Result<PathBuf> {
    let addr = std::env::var("GREETD_SOCK")
        .wrap_err("failed to read GREETD_SOCK from env")
        .suggestion(
            "Greetd must be running for Impolite to work. You might already be logged in.",
        )?;
    let addr = addr.parse()?;
    Ok(addr)
}

pub fn greetd_connect() -> Result<UnixStream> {
    let addr = greetd_socket_addr()?;
    let conn = UnixStream::connect(addr)?;
    Ok(conn)
}

pub fn greetd_decode(transport: &mut impl Read) -> Result<Response> {
    let mut len_buf = [0u8; 4];
    transport.read_exact(&mut len_buf)?;
    let len = u32::from_ne_bytes(len_buf);
    let mut buf = vec![0u8; len as _];
    transport.read_exact(&mut buf)?;
    greetd_decode_impl(&buf)
}

fn greetd_decode_impl(bytes: &[u8]) -> Result<Response> {
    let string = std::str::from_utf8(bytes)?;
    let res = serde_json::from_str(string)?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use crate::greetd::{Request, Response};

    #[test]
    fn serialize_messages() -> color_eyre::Result<()> {
        let msg = Request::CreateSession {
            username: "Bingus".into(),
        };

        assert_eq!(
            serde_json::to_string(&msg)?,
            r#"{"type":"create_session","username":"Bingus"}"#
        );

        let msg = Response::Success;

        assert_eq!(serde_json::to_string(&msg)?, r#"{"type":"success"}"#);

        Ok(())
    }
}
