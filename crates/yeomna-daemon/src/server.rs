//! The listener and the connection loop (spec 018).
//!
//! The socket is the trust boundary and the kernel enforces it. At
//! accept the peer's uid comes from `SO_PEERCRED`, which the peer
//! cannot set, and it becomes the actor for every audit row that
//! connection produces. Nothing in a frame says who is calling, so
//! there is nothing to spoof (V3).
//!
//! One `Session` per connection. That grain is what makes the session's
//! serialization and its retirement rules a property of one client
//! rather than of the appliance: a client that leaves a role escalation
//! unproven retires its own session and no one else's.

use std::path::Path;

use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};
use yeomna_verbs::frame::{self, FrameError};
use yeomna_verbs::{Session, Verb, actor, error_envelope};

/// Where the store is, and where to listen. Passed in rather than read
/// here, since the config file belongs to the appliance and not to this
/// module.
#[derive(Debug, Clone)]
pub struct Settings {
    pub socket_dir: String,
    pub port: u16,
    pub database: String,
    pub graph: Option<String>,
    /// Where the in-box embedder listens (spec 022). Handed to every
    /// session, which connects per call rather than holding a client, so
    /// the daemon still serves when the embedder is down.
    pub embedder_socket: String,
}

/// Bind the socket, with the mode the appliance model wants.
///
/// A stale socket from a crash is replaced rather than refused: the
/// alternative is an appliance that will not start after a power cut,
/// and the directory's own 0700 is what keeps a stranger from planting
/// one (FR7).
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        // EC-5: refuse to start rather than start and serve nothing.
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("the socket directory {} does not exist", parent.display()),
        ));
    }
    match std::fs::remove_file(path) {
        Ok(()) => warn!(path = %path.display(), "replaced a stale socket"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(path)?;
    // 0600: the single-operator appliance, where filesystem permission
    // is the gate the peercred check then names.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Accept forever, one task per connection.
pub async fn serve(listener: UnixListener, settings: Settings) {
    info!("yeomnad is listening");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let settings = settings.clone();
                tokio::spawn(async move {
                    if let Err(e) = connection(stream, settings).await {
                        debug!(%e, "connection ended");
                    }
                });
            }
            Err(e) => {
                error!(%e, "accept failed");
                return;
            }
        }
    }
}

/// One client: name it from the kernel, then answer frames until it
/// goes away.
async fn connection(mut stream: UnixStream, settings: Settings) -> Result<(), FrameError> {
    // V3 and D6. The uid is the kernel's answer, and a uid this machine
    // cannot name is a caller the appliance declines to serve: an audit
    // row that cannot say who acted is not a record, and admitting a
    // stranger under a synthetic name would make one.
    let Ok(peer) = stream.peer_cred() else {
        warn!("refused a peer whose credentials could not be read");
        return Ok(());
    };
    let uid = peer.uid();
    let Some(who) = actor::name_for_uid(uid) else {
        warn!(
            uid,
            "refused a peer whose uid this machine cannot name (D6)"
        );
        return Ok(());
    };
    info!(actor = %who, uid, "accepted");

    // EC-6: the store may be down, and a client deserves an envelope
    // saying so rather than a closed connection, with the daemon still
    // up for when it returns.
    let session = match yeomna_store::connect(
        &settings.socket_dir,
        settings.port,
        "yeomna_app",
        &settings.database,
    )
    .await
    {
        Ok(client) => {
            let mut s = Session::new(client, who.clone())
                .with_endpoint(settings.socket_dir.clone(), settings.port)
                .with_embedder(settings.embedder_socket.clone());
            if let Some(g) = settings.graph.clone() {
                s = s.with_graph(g);
            }
            Some(s)
        }
        Err(e) => {
            error!(%e, "the store is unreachable, answering with internal errors");
            None
        }
    };

    loop {
        let body = match frame::read(&mut stream).await {
            Ok(b) => b,
            Err(FrameError::Closed) => return Ok(()),
            Err(e) => return Err(e),
        };
        let response = answer(session.as_ref(), &body).await;
        frame::write(&mut stream, response.as_bytes()).await?;
    }
}

/// One request to one envelope, always. A caller that framed correctly
/// gets an answer in the shape it expects, even when what it framed was
/// not a verb (FR5).
async fn answer(session: Option<&Session>, body: &[u8]) -> String {
    let envelope = match serde_json::from_slice::<Verb>(body) {
        Ok(verb) => match session {
            Some(s) => s.call(&verb).await,
            None => error_envelope(
                verb.wire_name(),
                &yeomna_verbs::VerbError::Internal("the store is unreachable".into()),
            ),
        },
        Err(e) => error_envelope(
            "unknown",
            &yeomna_verbs::VerbError::InvalidArgs(format!("not a verb request: {e}")),
        ),
    };
    serde_json::to_string(&envelope).unwrap_or_else(|e| {
        // The verb answered and its answer will not serialize, which is
        // this crate's fault. Say so in the shape the caller parses.
        format!(
            "{{\"success\":false,\"command\":\"unknown\",\"error\":\"internal: cannot render the envelope: {e}\",\"timestamp\":\"\"}}"
        )
    })
}
