//! End-to-end browser tests for the AMATL UI.
//!
//! The UI is 32 KB of vanilla JavaScript served as static assets, so every
//! contract that matters — that a search renders results, that the page is
//! keyboard-navigable, that the empty and degraded states are reachable — is
//! only observable in a real browser. Nothing below stubs the DOM.
//!
//! These tests are gated exactly like `trafilatura_integration`: without
//! `AMATL_BROWSER_E2E=1` they are inert, so a checkout without a driver still
//! passes `cargo test --workspace`. Once opted in, they are **fail-closed**: a
//! missing, unreachable, or version-mismatched WebDriver fails the run instead
//! of skipping green — a mismatch (e.g. a chromedriver that cannot drive the
//! installed Chrome) must never look like a pass.
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

/// Connect a headless browser for an opted-in run.
///
/// Only called once [`enabled`] is true, i.e. the run declared it wants real
/// browser coverage. At that point a missing or unusable driver is a **hard
/// failure**, not a skip: an absent driver, a driver that fails to create a
/// session, or — the case this guards — a driver whose version cannot drive the
/// installed browser, must fail the run instead of reporting a green skip while
/// testing nothing. A driver-less checkout stays green by never opting in
/// (no `AMATL_BROWSER_E2E=1`), which is the only legitimate skip.
async fn browser() -> Client {
    let mut options = serde_json::Map::new();
    options.insert(
        "args".into(),
        serde_json::json!(["--headless=new", "--no-sandbox", "--disable-gpu"]),
    );
    // Chromedriver discovers the browser through the platform's default path,
    // which on a CI runner is the *preinstalled* Chrome — not necessarily the
    // version the job pinned. When the workflow installs a specific Chrome and
    // points `AMATL_BROWSER_BIN` at it, launch that binary so the pair the job
    // verified is the pair the tests drive.
    if let Ok(binary) = std::env::var("AMATL_BROWSER_BIN") {
        if !binary.is_empty() {
            options.insert("binary".into(), serde_json::Value::String(binary));
        }
    }
    let mut capabilities = serde_json::Map::new();
    capabilities.insert(
        "goog:chromeOptions".into(),
        serde_json::Value::Object(options),
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
        Ok(client) => client,
        Err(error) => panic!(
            "AMATL_BROWSER_E2E=1 was set but no usable WebDriver could be reached at {}: {error}. \
             Start a chromedriver matching the installed Chrome (a version mismatch reports \
             'cannot find Chrome binary' / 'only supports Chrome version N'), or unset \
             AMATL_BROWSER_E2E to skip the browser suite entirely.",
            webdriver_url()
        ),
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
    let client = browser().await;

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
    let client = browser().await;

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
    let client = browser().await;

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
    let client = browser().await;

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
    let client = browser().await;

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

#[tokio::test]
async fn synthetic_mode_hides_real_provider_controls_and_no_auth_token_field() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let client = browser().await;
    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    let mut state = serde_json::Value::Null;
    for _ in 0..100 {
        state = client
            .execute(
                "return JSON.stringify({ready: !document.querySelector('.service-state').hidden, accessHidden: document.querySelector('.access-filters').hidden, providersHidden: document.querySelector('.provider-status').hidden, credentialsHidden: document.querySelector('.source-credentials').hidden, queryWidth: document.querySelector('#query').getBoundingClientRect().width, clientsCollapsed: !document.querySelector('.clients-panel').open, operatorToolsCollapsed: !document.querySelector('.operator-tools').open, answerDisabled: document.querySelector('#answer-button').disabled, answerHidden: document.querySelector('#answer-button').hidden, answerHintHidden: document.querySelector('#answer-hint').hidden});",
                vec![],
            )
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(state.as_str().unwrap()).unwrap();
        if parsed["ready"] == true {
            state = parsed;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let parsed = if state.is_string() {
        serde_json::from_str::<serde_json::Value>(state.as_str().unwrap()).unwrap()
    } else {
        state
    };
    assert_eq!(parsed["ready"], true);
    assert_eq!(parsed["accessHidden"], true);
    assert_eq!(parsed["providersHidden"], true);
    assert_eq!(parsed["credentialsHidden"], true);
    assert!(parsed["queryWidth"].as_f64().unwrap() > 200.0);
    assert_eq!(parsed["clientsCollapsed"], true);
    assert_eq!(parsed["operatorToolsCollapsed"], true);
    assert_eq!(parsed["answerDisabled"], false);
    assert_eq!(parsed["answerHidden"], true);
    assert_eq!(parsed["answerHintHidden"], true);

    client.close().await.unwrap();
}

#[tokio::test]
async fn switching_interface_language_updates_language_and_primary_copy() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let client = browser().await;

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    client
        .execute(
            "const select = document.getElementById('ui-language'); \
             select.value = 'es'; \
             select.dispatchEvent(new Event('change', { bubbles: true }));",
            vec![],
        )
        .await
        .unwrap();

    let language = client
        .execute("return document.documentElement.lang;", vec![])
        .await
        .unwrap();
    assert_eq!(language.as_str(), Some("es"));

    let heading = client
        .find(Locator::Id("search-heading"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(heading, "Busca y verifica fuentes");

    let mode_help = client
        .find(Locator::Id("search-mode-help"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(mode_help.contains("Buscar"));

    client.close().await.unwrap();
}

/// The `hidden` attribute is the single mechanism the JS uses to show and hide
/// panels, but a component that sets its own `display:` (flex / inline-flex /
/// inline-block) beats the UA `[hidden] { display: none }` unless the author
/// stylesheet restores `hidden`. The source selector, the Marginalia credential
/// row, and a fragment's signal chips are all toggled this way, so each must
/// compute to `display: none` the instant `element.hidden` is true. This runs in
/// a real browser because only the computed style reveals the cascade winner.
#[tokio::test]
async fn hidden_attribute_collapses_the_panels_it_hides() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let client = browser().await;

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    // Open the collapsed filter `<details>` and force-render a Deep fragment so
    // every probed element exists in the live DOM (fragment chips start inside
    // a `<template>`, which is not a rendered node).
    client
        .execute(
            "document.querySelector('details.filters').open = true; \
             const tpl = document.getElementById('fragment-template').content.cloneNode(true); \
             document.body.appendChild(tpl);",
            vec![],
        )
        .await
        .unwrap();

    let report = client
        .execute(
            "const selectors = { \
                 credentialActions: '#credential-actions', \
                 marginaliaGroup: '#marginalia-credential-group', \
                 fragmentSignals: '.fragment-signals' \
             }; \
             const out = {}; \
             for (const [name, sel] of Object.entries(selectors)) { \
                 const el = document.querySelector(sel); \
                 if (!el) { out[name] = 'missing'; continue; } \
                 const before = el.hidden; \
                 el.hidden = true; \
                 out[name] = getComputedStyle(el).display; \
                 el.hidden = before; \
             } \
             return JSON.stringify(out);",
            vec![],
        )
        .await
        .unwrap();
    let computed = report.as_str().unwrap_or("{}");
    let parsed: serde_json::Value = serde_json::from_str(computed).unwrap_or_default();
    for name in ["credentialActions", "marginaliaGroup", "fragmentSignals"] {
        assert_eq!(
            parsed.get(name).and_then(|value| value.as_str()),
            Some("none"),
            "{name} stayed visible while hidden=true (computed: {computed})"
        );
    }

    client.close().await.unwrap();
}

/// With no strategist configured there is nothing to enable, so the whole
/// "model" row (`#model-options`) must stay collapsed instead of presenting a
/// dead checkbox under the search box. `spawn_server` builds the default config,
/// where the strategist has no endpoint/model, so `/status` reports
/// `configured: false` and the JS hides the row.
///
/// This runs in a real browser because only the computed style proves the hide
/// actually engaged: the row sets `display: flex`, which beats the UA
/// `[hidden] { display: none }` unless the author stylesheet restores it — and
/// an element that is `hidden` but still paints is exactly the regression this
/// guards.
#[tokio::test]
async fn model_toggle_is_hidden_when_the_strategist_is_not_configured() {
    if !enabled() {
        return;
    }
    let port = spawn_server().await;
    let client = browser().await;

    client
        .goto(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();

    // `/status` resolves asynchronously after load, so poll for the JS to apply
    // the gate rather than reading the first paint.
    wait_for(&client, Locator::Css("#model-options[hidden]")).await;

    let report = client
        .execute(
            "const el = document.getElementById('model-options'); \
             const toggle = document.getElementById('strategist-enabled-toggle'); \
             return JSON.stringify({ \
                 hidden: el.hidden, \
                 display: getComputedStyle(el).display, \
                 height: el.offsetHeight, \
                 toggleDisabled: toggle.disabled, \
                 toggleChecked: toggle.checked \
             });",
            vec![],
        )
        .await
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(report.as_str().unwrap_or("{}")).unwrap_or_default();

    assert_eq!(
        parsed.get("hidden").and_then(|v| v.as_bool()),
        Some(true),
        "the model row stayed visible with the strategist unconfigured: {parsed}"
    );
    assert_eq!(
        parsed.get("display").and_then(|v| v.as_str()),
        Some("none"),
        "the model row is hidden but still paints (display: flex wins): {parsed}"
    );
    assert_eq!(
        parsed.get("height").and_then(|v| v.as_i64()),
        Some(0),
        "the model row occupies layout space while hidden: {parsed}"
    );
    assert_eq!(
        parsed.get("toggleDisabled").and_then(|v| v.as_bool()),
        Some(true),
        "the toggle should be disabled with the strategist unconfigured: {parsed}"
    );
    assert_eq!(
        parsed.get("toggleChecked").and_then(|v| v.as_bool()),
        Some(false),
        "a disabled toggle must never read as switched on: {parsed}"
    );

    client.close().await.unwrap();
}
