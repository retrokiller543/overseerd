//! Back-compat coverage for the names renamed in 0.7.0 (`Daemon`/`DaemonBuilder`/`daemon!`
//! → `App`/`AppBuilder`/`app!`). The aliases are removed in 1.0.0; until then they must keep
//! compiling and behaving exactly like the new names. `#![allow(deprecated)]` keeps the suite
//! warning-free while still exercising the deprecated surface.
#![allow(deprecated)]

use overseerd::daemon::{Daemon, DaemonBuilder, daemon};

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

#[tokio::test]
async fn daemon_macro_alias_builds() {
    let app = daemon! { name: "deprecated-macro-alias" }
        .build()
        .await
        .expect("daemon! alias builds");

    assert_eq!(app.name, "deprecated-macro-alias");
}
