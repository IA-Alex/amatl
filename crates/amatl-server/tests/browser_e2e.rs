//! End-to-end browser tests for the AMATL UI.
//!
//! The UI is 32 KB of vanilla JavaScript served as static assets, so every
//! contract that matters — that a search renders results, that the page is
//! keyboard-navigable, that the empty and degraded states are reachable — is
//! only observable in a real browser. Nothing below stubs the DOM.
//!
//! These tests are gated exactly like `trafilatura_integration`: they are inert
//! unless `AMATL_BROWSER_E2E=1` and a WebDriver endpoint are both present, so a
//! checkout without a driver still passes `cargo test --workspace`.
//!
//! Running them locally:
//!
//! ```text
//! chromedriver --port=4444 &
//! AMATL_BROWSER_E2E=1 cargo test -p amatl-server --test browser_e2e
//! ```
//!
//! The server is bound to loopback and runs with `--mock`, so no test here
//! reaches the network: the browser only ever talks to this process.

use amatl_core::AmatlService;
use amatl_server::serve;
use fantoccini::{Client, ClientBuilder, Locator};
use std::time::Duration;
use tokio::net::TcpStream;

/// WebDriver endpoint; chromedriver's default.
fn webdriver_url() -> String {
    std::env::var("AMATL_WEBDRIVER_URL").unwrap_or_else(|_| "http://localhost:4444".into())
}

/// Whether this run is allowed to drive a browser.
fn enabled() -> bool {
    std::env::var("AMATL_BROWSER_E2E").as_deref() == Ok("1")
}

/// Start the server on an ephemeral loopback port with mock providers.
async fn spawn_server() -> u16 {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let mut config = amatl_core::Config::default();
    config.server.port = port;
    // The UI is served without a bearer; the API calls it makes are not. Auth
    // is disabled so the test exercises the interface, not the token flow,
    // which `tests.rs` already covers.
    config.server.no_auth = true;
    config.server.rate_limit_per_minute = 1_000_000;

    let service = AmatlService::new(config, true).await;
    tokio::spawn(serve(service));

    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("server did not start on port {port}");
}

/// Connect a headless browser, or skip when no driver is reachable.
async fn browser() -> Option<Client> {
    let mut capabilities = serde_json::Map::new();
    capabilities.insert(
        "goog:chromeOptions".into(),
        serde_json::json!({
            "args": ["--headless=new", "--no-sandbox", "--disable-gpu"]
        }),
    );
    // A plain HTTP connector, not a TLS one: WebDriver runs on loopback, and
    // `ClientBuilder::rustls()` would additionally require a process-wide
    // rustls CryptoProvider to be installed, which this workspace leaves to
    // the binary rather than to a test harness.
    let connector = hyper_util::client::legacy::connect::HttpConnector::new();
    match ClientBuilder::new(connector)
        .capabilities(capabilities)
        .connect(&webdriver_url())
        .await
    {
        Ok(client) => Some(client),
        Err(error) => {
            eprintln!("skipping: no WebDriver at {}: {error}", webdriver_url());
            None
        }
    }
}

/// Poll until `locator` matches, so tests do not race the async render.
async fn wait_for(client: &Client, locator: Locator<'_>) -> fantoccini::elements::Element {
    for _ in 0..100 {
        if let Ok(element) = client.find(locator).await {
            return element;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("element never appeared: {locator:?}");
}

#[tokio::test]
async fn search_renders_results_in_a_real_browser() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let Some(client) = browser().await else {
        return;
    };

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    client
        .find(Locator::Id("query"))
        .await
        .unwrap()
        .send_keys("rust async")
        .await
        .unwrap();
    client
        .find(Locator::Id("search-button"))
        .await
        .unwrap()
        .click()
        .await
        .unwrap();

    let results = wait_for(&client, Locator::Css("#results li")).await;
    let text = results.text().await.unwrap();
    assert!(!text.trim().is_empty(), "a result rendered with no text");

    // The UI must never surface the fetch frontier for a plain search.
    let body = client.source().await.unwrap();
    assert!(
        !body.contains("final_url"),
        "search must not expose final_url"
    );

    client.close().await.unwrap();
}

#[tokio::test]
async fn the_search_flow_is_reachable_by_keyboard_alone() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let Some(client) = browser().await else {
        return;
    };

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    // Typing into the query field and pressing Enter must submit the form,
    // without the pointer ever being used.
    let query = client.find(Locator::Id("query")).await.unwrap();
    query.send_keys("rust async").await.unwrap();
    query.send_keys("\u{E007}").await.unwrap(); // Enter

    let results = wait_for(&client, Locator::Css("#results li")).await;
    assert!(!results.text().await.unwrap().trim().is_empty());

    client.close().await.unwrap();
}

#[tokio::test]
async fn an_empty_result_set_renders_its_own_state() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let Some(client) = browser().await else {
        return;
    };

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    let query = client.find(Locator::Id("query")).await.unwrap();
    // A query the mock corpus cannot match.
    query
        .send_keys("zzzzqqqq-no-such-corpus-token")
        .await
        .unwrap();
    query.send_keys("\u{E007}").await.unwrap();

    // The status region must say something rather than leaving a blank page.
    let status = wait_for(&client, Locator::Id("status")).await;
    for _ in 0..100 {
        if !status.text().await.unwrap_or_default().trim().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !status.text().await.unwrap().trim().is_empty(),
        "an empty result set left the status region blank"
    );

    client.close().await.unwrap();
}

/// axe-core 4.13.0, vendored — see `fixtures/axe-core/NOTICE.md` for
/// provenance, license and upgrade instructions.
const AXE_CORE_SOURCE: &str = include_str!("fixtures/axe-core/axe.min.js");

#[tokio::test]
async fn the_ui_has_no_automatically_detectable_accessibility_violations() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let Some(client) = browser().await else {
        return;
    };

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    // Populate the results and status regions before auditing: the idle page
    // and the results page are different DOM states, and both matter.
    let query = client.find(Locator::Id("query")).await.unwrap();
    query.send_keys("rust async").await.unwrap();
    query.send_keys("\u{E007}").await.unwrap(); // Enter
    wait_for(&client, Locator::Css("#results li")).await;

    // `execute` runs through the WebDriver "Execute Script" command (CDP
    // `Runtime.evaluate` under Chrome), not a `<script>` tag the page parses
    // itself — it is not subject to the page's own `script-src 'self'` CSP,
    // the same way the DevTools console isn't. This assertion is the load-
    // bearing part of the test: if axe-core fails to define `window.axe`
    // here, CSP (or something else) blocked injection and the run below is
    // meaningless.
    client.execute(AXE_CORE_SOURCE, vec![]).await.unwrap();
    let axe_defined: bool = client
        .execute("return typeof window.axe !== 'undefined';", vec![])
        .await
        .unwrap()
        .as_bool()
        .unwrap_or(false);
    assert!(
        axe_defined,
        "axe-core did not load — script injection was blocked (CSP or otherwise)"
    );

    // `axe.run()` is promise-based; `execute_async` supplies the completion
    // callback as the last element of `arguments`.
    let report = client
        .execute_async(
            "var callback = arguments[arguments.length - 1]; \
             axe.run(document, {}).then( \
                 function(results) { callback(results.violations); }, \
                 function(error) { callback([{ id: 'axe-run-failed', description: String(error) }]); } \
             );",
            vec![],
        )
        .await
        .unwrap();

    let violations = report.as_array().cloned().unwrap_or_default();
    assert!(
        violations.is_empty(),
        "axe-core reported accessibility violations: {}",
        serde_json::to_string_pretty(&violations).unwrap_or_default()
    );

    client.close().await.unwrap();
}

#[tokio::test]
async fn the_layout_holds_at_a_narrow_viewport() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let Some(client) = browser().await else {
        return;
    };

    client.set_window_size(360, 720).await.unwrap();
    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    // The page must not scroll horizontally at a phone width.
    let overflows: bool = client
        .execute(
            "return document.documentElement.scrollWidth > document.documentElement.clientWidth;",
            vec![],
        )
        .await
        .unwrap()
        .as_bool()
        .unwrap_or(true);
    assert!(!overflows, "the page scrolls horizontally at 360px");

    client.close().await.unwrap();
}
