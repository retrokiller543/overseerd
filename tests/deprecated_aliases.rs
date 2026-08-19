//! Back-compat coverage for the type names renamed in 0.7.0 (`Daemon`/`DaemonBuilder` to
//! `App`/`AppBuilder`). The aliases are removed in 1.0.0; until then they must keep compiling and
//! behaving exactly like the new names. `#![allow(deprecated)]` keeps the suite warning-free while
//! still exercising the deprecated surface.
#![cfg(feature = "daemon")]
#![allow(deprecated)]

use upwell::daemon::{Daemon, DaemonBuilder};

#[tokio::test]
async fn daemon_type_alias_builds() {
    let app = Daemon::builder("deprecated-type-alias")
        .build()
        .await
        .expect("Daemon alias builds");

    assert_eq!(app.name, "deprecated-type-alias");
}

#[tokio::test]
async fn daemon_builder_alias_builds() {
    let app = DaemonBuilder::new("deprecated-builder-alias")
        .build()
        .await
        .expect("DaemonBuilder alias builds");

    assert_eq!(app.name, "deprecated-builder-alias");
}
