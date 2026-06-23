// `option_env!` in src/config.rs bakes OAuth client config into release builds.
// Tell Cargo to rebuild this crate when those build-time vars change, so the
// embedded values can't go stale across incremental builds.
fn main() {
    for key in [
        "MAILAGENT_GOOGLE_CLIENT_ID",
        "MAILAGENT_GOOGLE_CLIENT_SECRET",
        "MAILAGENT_MICROSOFT_CLIENT_ID",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
}
