//! First-run runtime install: download the pinned Node + `npm ci` codex, then reuse
//! on a second call.
//!
//! Gated behind `RUN_INTEGRATION_TESTS` because it hits the network (downloads
//! ~30 MB of Node and runs `npm ci`). The fast, network-free checks for path
//! resolution / target mapping live as unit tests in `src/runtime/mod.rs`.

use hi_agent::runtime;

#[tokio::test]
async fn installs_then_reuses() {
    if std::env::var_os("RUN_INTEGRATION_TESTS").is_none() {
        eprintln!("skipping: set RUN_INTEGRATION_TESTS=1 (downloads Node + runs npm ci)");
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
    assert!(
        r1.node_bin.exists(),
        "node missing: {}",
        r1.node_bin.display()
    );
    // The codex path is resolved by searching the installed tree for the platform
    // package's native binary, so this also pins that the layout is what we expect.
    assert!(
        r1.codex_bin.exists(),
        "codex missing: {}",
        r1.codex_bin.display()
    );
    assert!(
        !r1.codex_bin.extension().is_some_and(|e| e == "js"),
        "resolved the JS launcher rather than the native binary: {}",
        r1.codex_bin.display()
    );

    // Second call reuses the install (`.complete` marker present).
    let r2 = runtime::ensure().await.expect("reuse should succeed");
    assert_eq!(r1.node_bin, r2.node_bin);
    assert_eq!(r1.codex_bin, r2.codex_bin);
    assert_eq!(r1.node_modules, r2.node_modules);
}
