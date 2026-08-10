//! First-run runtime install: download + unpack the pinned codex and esbuild
//! tarballs, then reuse them on a second call.
//!
//! Gated behind `RUN_INTEGRATION_TESTS` because it hits the network (~135 MB of
//! platform tarballs from the npm registry). The fast, network-free checks for path
//! resolution / target mapping live as unit tests in `src/runtime/mod.rs`.

use hi_agent::runtime;

#[tokio::test]
async fn installs_then_reuses() {
    if std::env::var_os("RUN_INTEGRATION_TESTS").is_none() {
        eprintln!("skipping: set RUN_INTEGRATION_TESTS=1 (downloads ~135 MB of tarballs)");
        return;
    }

    let cache = tempfile::tempdir().unwrap();
    // Pin the install to a throwaway cache dir. Single-threaded test binary, so
    // this process-global env set is safe for this test.
    unsafe {
        std::env::set_var("HI_AGENT_RUNTIME_DIR", cache.path());
    }

    let r1 = runtime::ensure()
        .await
        .expect("first-run install should succeed");
    // A pin-matching `codex` on this host's PATH short-circuits to the system runtime,
    // which downloads nothing and so proves nothing about the install path.
    if r1.origin != "managed" {
        eprintln!("skipping: resolved a {} runtime, not a managed install", r1.origin);
        return;
    }

    // The codex path is resolved by searching the extracted tree for the native binary
    // under `vendor/<rust-target>/`, so this also pins that the layout is what we
    // expect — and that we didn't settle for a JS launcher.
    assert!(
        r1.codex_bin.exists(),
        "codex missing: {}",
        r1.codex_bin.display()
    );
    assert!(
        !r1.codex_bin.extension().is_some_and(|e| e == "js"),
        "resolved a JS launcher rather than the native binary: {}",
        r1.codex_bin.display()
    );
    let esbuild = r1
        .esbuild_bin
        .as_deref()
        .expect("a managed runtime installs esbuild alongside codex");
    assert!(esbuild.exists(), "esbuild missing: {}", esbuild.display());

    // Nothing in the install may be an interpreter we then have to feed: the whole
    // point of fetching tarballs directly is that no Node tree is produced.
    assert!(
        !cache.path().join("node").exists(),
        "the install produced a node tree at {}",
        cache.path().join("node").display()
    );

    // Second call reuses the install (`.complete` marker present).
    let r2 = runtime::ensure().await.expect("reuse should succeed");
    assert_eq!(r1.codex_bin, r2.codex_bin);
    assert_eq!(r1.esbuild_bin, r2.esbuild_bin);
}
