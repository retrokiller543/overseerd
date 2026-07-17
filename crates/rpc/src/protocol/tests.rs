use std::io::{Error, ErrorKind};

use super::is_transient_accept_error;

#[test]
fn retries_only_plausibly_transient_accept_errors() {
    for kind in [
        ErrorKind::Interrupted,
        ErrorKind::WouldBlock,
        ErrorKind::ConnectionAborted,
        ErrorKind::ConnectionReset,
        ErrorKind::TimedOut,
    ] {
        assert!(is_transient_accept_error(&Error::from(kind)));
    }

    #[cfg(windows)]
    let transient_raw = [4, 8, 14, 10024, 10055].as_slice();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let transient_raw = [12, 23, 24, 105].as_slice();
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    let transient_raw = [12, 23, 24, 55].as_slice();
    #[cfg(all(
        unix,
        not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))
    ))]
    let transient_raw = [12, 23, 24].as_slice();
    #[cfg(not(any(unix, windows)))]
    let transient_raw = [].as_slice();

    for raw in transient_raw {
        assert!(is_transient_accept_error(&Error::from_raw_os_error(*raw)));
    }

    #[cfg(unix)]
    for raw in [8, 14, 10024, 10055] {
        assert!(!is_transient_accept_error(&Error::from_raw_os_error(raw)));
    }

    #[cfg(windows)]
    for raw in [12, 23, 24, 105] {
        assert!(!is_transient_accept_error(&Error::from_raw_os_error(raw)));
    }

    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::InvalidInput,
        ErrorKind::AddrNotAvailable,
        ErrorKind::Unsupported,
        ErrorKind::Other,
    ] {
        assert!(!is_transient_accept_error(&Error::from(kind)));
    }
}
