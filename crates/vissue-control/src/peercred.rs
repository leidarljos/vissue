//! Same-uid check for an accepted Unix socket. No `unsafe` in this crate.

use std::io;
use std::os::fd::AsFd;

use nix::unistd::Uid;

/// Accept the peer when its uid matches the current process uid.
#[must_use]
pub fn accept_peer(uid: u32) -> bool {
    uid == current_uid()
}

/// Real uid of this process (`getuid`), matching `Uid::current`.
#[must_use]
pub fn current_uid() -> u32 {
    Uid::current().as_raw()
}

/// Peer uid from `SO_PEERCRED` on Linux, or `getpeereid` on BSD/macOS.
///
/// # Errors
///
/// Returns an error when the socket option cannot be read, or this OS cannot
/// report peer credentials.
pub fn peer_uid<F: AsFd>(sock: &F) -> io::Result<u32> {
    peer_uid_impl(sock)
}

/// Label when the peer uid does not match, or credentials cannot be read.
///
/// `Ok` of this process, and `ErrorKind::Unsupported`, are quiet. An OS that
/// cannot report a uid still relies on directory mode 0700 and socket mode
/// 0600. A mismatch or any other IO error is a warning, not a reason to
/// drop the connection: [`accept_from_result`] stays open.
#[must_use]
pub fn peer_label(result: io::Result<u32>) -> Option<String> {
    match result {
        Ok(uid) if accept_peer(uid) => None,
        Ok(uid) => Some(format!(
            "[warn] peer uid={uid} is not this process; serving the connection"
        )),
        Err(err) if err.kind() == io::ErrorKind::Unsupported => None,
        Err(err) => Some(format!(
            "[warn] peer credentials unreadable ({err}); serving the connection"
        )),
    }
}

/// Whether an accepted socket may stay open.
///
/// Always. A uid mismatch or an unreadable credential is [`peer_label`];
/// the connection is still served. `Unsupported` has no label: dir 0700
/// and sock 0600 are the check on that OS.
#[must_use]
pub fn accept_from_result(_result: io::Result<u32>) -> bool {
    true
}

/// [`accept_from_result`] over [`peer_uid`].
#[must_use]
pub fn accept_socket<F: AsFd>(sock: &F) -> bool {
    accept_from_result(peer_uid(sock))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid_impl<F: AsFd>(sock: &F) -> io::Result<u32> {
    use nix::sys::socket::{getsockopt, sockopt};
    let creds = getsockopt(sock, sockopt::PeerCredentials)?;
    Ok(creds.uid())
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
fn peer_uid_impl<F: AsFd>(sock: &F) -> io::Result<u32> {
    let (uid, _) = nix::unistd::getpeereid(sock)?;
    Ok(uid.as_raw())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
fn peer_uid_impl<F: AsFd>(_sock: &F) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "peer credentials are unavailable; rely on dir 0700 / sock 0600",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_uid_is_accepted() {
        assert!(accept_peer(current_uid()));
    }

    #[test]
    fn other_uid_is_served_and_labeled() {
        let other = current_uid().wrapping_add(1);
        assert!(!accept_peer(other));
        assert!(accept_from_result(Ok(other)));
        let label = peer_label(Ok(other)).expect("a foreign uid is labeled");
        assert!(label.starts_with("[warn]"), "{label}");
        assert!(label.contains(&other.to_string()), "{label}");
        assert!(label.contains("serving the connection"), "{label}");
        assert!(peer_label(Ok(current_uid())).is_none());
    }

    #[test]
    fn unsupported_peercred_stays_quiet_and_an_unreadable_one_is_labeled() {
        let err = io::Error::new(io::ErrorKind::Unsupported, "no SO_PEERCRED");
        assert!(peer_label(Err(err)).is_none());
        assert!(accept_from_result(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no SO_PEERCRED"
        ))));
        let err = io::Error::other("getsockopt failed");
        let label = peer_label(Err(err)).expect("an unreadable credential is labeled");
        assert!(label.starts_with("[warn]"), "{label}");
        assert!(label.contains("serving the connection"), "{label}");
        assert!(accept_from_result(Err(io::Error::other(
            "getsockopt failed"
        ))));
        assert!(accept_from_result(Ok(current_uid())));
    }

    #[test]
    fn same_process_peer_is_accepted() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peer.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let client = UnixStream::connect(&path).unwrap();
        let (server, _) = listener.accept().unwrap();
        let uid = peer_uid(&server).unwrap();
        assert_eq!(uid, current_uid());
        assert!(accept_peer(uid));
        assert!(accept_socket(&server));
        assert!(accept_socket(&client));
    }
}
