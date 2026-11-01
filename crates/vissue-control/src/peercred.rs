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

/// Whether an accepted socket may stay open.
///
/// `Ok(uid)` uses [`accept_peer`]. `ErrorKind::Unsupported` means this OS
/// cannot read peer credentials: return true so dir 0700 / sock 0600 are the
/// check. Every other IO error and a uid mismatch fail closed.
#[must_use]
pub fn accept_from_result(result: io::Result<u32>) -> bool {
    match result {
        Ok(uid) => accept_peer(uid),
        Err(err) if err.kind() == io::ErrorKind::Unsupported => true,
        Err(_) => false,
    }
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
    fn other_uid_is_rejected() {
        let other = current_uid().wrapping_add(1);
        assert!(!accept_peer(other));
        assert!(!accept_from_result(Ok(other)));
    }

    #[test]
    fn unsupported_peercred_falls_back_to_mode_bits() {
        let err = io::Error::new(io::ErrorKind::Unsupported, "no SO_PEERCRED");
        assert!(accept_from_result(Err(err)));
        let err = io::Error::other("getsockopt failed");
        assert!(!accept_from_result(Err(err)));
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
