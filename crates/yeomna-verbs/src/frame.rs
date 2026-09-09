//! The wire format: a four-byte big-endian length, then that many bytes
//! of JSON (spec 018, R21 D5).
//!
//! The length prefix is the one field a caller controls before any
//! parsing happens, so it is checked before it is honored. A claim
//! larger than the maximum is refused without allocating what it asked
//! for, which is the difference between a bounded server and a caller's
//! memory budget.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The largest frame either direction, per R21 D5. Generous for a verb
/// request and for any envelope the read verbs produce under their own
/// page caps, and small enough that a hostile length is cheap to refuse.
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// Why a frame did not arrive or could not leave.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The peer closed cleanly between frames, which is how a client
    /// says it is done and not an error to log loudly (EC-1).
    #[error("the peer closed")]
    Closed,
    /// The peer vanished mid-frame (EC-2).
    #[error("the frame ended short: {0}")]
    Short(std::io::Error),
    /// A length larger than this server will honor (FR4).
    #[error("a frame of {claimed} bytes was claimed, the maximum is {MAX_FRAME}")]
    TooLarge { claimed: u32 },
    #[error("transport error: {0}")]
    Io(std::io::Error),
}

/// Read one frame, or say why not.
pub async fn read<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::Closed);
        }
        Err(e) => return Err(FrameError::Io(e)),
    }
    let claimed = u32::from_be_bytes(len);
    // Checked before it is honored: nothing of this size is allocated
    // until the size itself is admissible.
    if claimed > MAX_FRAME {
        return Err(FrameError::TooLarge { claimed });
    }
    let mut body = vec![0u8; claimed as usize];
    r.read_exact(&mut body).await.map_err(FrameError::Short)?;
    Ok(body)
}

/// Write one frame.
pub async fn write<W: AsyncWriteExt + Unpin>(w: &mut W, body: &[u8]) -> Result<(), FrameError> {
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge { claimed: u32::MAX })?;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge { claimed: len });
    }
    w.write_all(&len.to_be_bytes())
        .await
        .map_err(FrameError::Io)?;
    w.write_all(body).await.map_err(FrameError::Io)?;
    w.flush().await.map_err(FrameError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_frame_round_trips() {
        let mut buf = Vec::new();
        write(&mut buf, b"{\"verb\":\"status\",\"args\":{}}")
            .await
            .unwrap();
        assert_eq!(&buf[..4], &27u32.to_be_bytes(), "the length leads");
        let mut cursor = std::io::Cursor::new(buf);
        let body = read(&mut cursor).await.unwrap();
        assert_eq!(body, b"{\"verb\":\"status\",\"args\":{}}");
    }

    #[tokio::test]
    async fn many_frames_read_in_sequence() {
        let mut buf = Vec::new();
        for n in 0..3 {
            write(&mut buf, format!("frame {n}").as_bytes())
                .await
                .unwrap();
        }
        let mut cursor = std::io::Cursor::new(buf);
        for n in 0..3 {
            assert_eq!(
                read(&mut cursor).await.unwrap(),
                format!("frame {n}").as_bytes()
            );
        }
        assert!(matches!(read(&mut cursor).await, Err(FrameError::Closed)));
    }

    /// FR4: the claim is refused before anything of that size exists.
    #[tokio::test]
    async fn an_oversized_claim_is_refused_without_being_honored() {
        let mut buf = (MAX_FRAME + 1).to_be_bytes().to_vec();
        buf.extend_from_slice(b"a few real bytes");
        let mut cursor = std::io::Cursor::new(buf);
        match read(&mut cursor).await {
            Err(FrameError::TooLarge { claimed }) => assert_eq!(claimed, MAX_FRAME + 1),
            other => panic!("expected a refusal, got {other:?}"),
        }
        // And the maximum itself is admissible as a claim, so the bound
        // is a limit rather than an off-by-one.
        let mut at_max = MAX_FRAME.to_be_bytes().to_vec();
        at_max.extend_from_slice(b"short");
        let mut cursor = std::io::Cursor::new(at_max);
        assert!(
            matches!(read(&mut cursor).await, Err(FrameError::Short(_))),
            "a legal claim fails on the body, not on the length"
        );
    }

    /// EC-2: half a frame reaches no verb.
    #[tokio::test]
    async fn a_truncated_frame_is_short_rather_than_partial() {
        let mut buf = 100u32.to_be_bytes().to_vec();
        buf.extend_from_slice(b"only ten b");
        let mut cursor = std::io::Cursor::new(buf);
        assert!(matches!(read(&mut cursor).await, Err(FrameError::Short(_))));
    }

    /// EC-1: a clean close between frames is how a client says it is
    /// done, and is not an error worth shouting about.
    #[tokio::test]
    async fn a_clean_close_is_its_own_answer() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        assert!(matches!(read(&mut cursor).await, Err(FrameError::Closed)));
    }
}
