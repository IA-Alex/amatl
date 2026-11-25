"use strict";

// ── Theme ───────────────────────────────────────────────────────
const THEME_KEY = "amatl-theme";
function storedTheme() {
  try {
    const value = localStorage.getItem(THEME_KEY);
    return value === "light" || value === "dark" ? value : null;
  } catch {
    return null;
  }
}
function applyTheme(theme) {
  if (theme === "light" || theme === "dark") {
    document.documentElement.setAttribute("data-theme", theme);
  } else {
    document.documentElement.removeAttribute("data-theme");
  }
}
applyTheme(storedTheme());

// ── i18n ────────────────────────────────────────────────────────
const CATALOG = globalThis.AMATL_LOCALES || { defaultLocale: "en", locales: { en: {} } };
const FALLBACK = CATALOG.locales[CATALOG.defaultLocale] || {};
const LANGUAGE_KEY = "amatl-language";

function catalogLocale(candidate) {
  const value = typeof candidate === "string" ? candidate.slice(0, 2).toLowerCase() : "";
  return Object.prototype.hasOwnProperty.call(CATALOG.locales, value) ? value : null;
}

function storedLocale() {
  try {
    return catalogLocale(localStorage.getItem(LANGUAGE_KEY));
  } catch {
    return null;
  }
}

function navigatorLocale() {
  return catalogLocale(typeof navigator !== "undefined" ? navigator.language : "")
    || CATALOG.defaultLocale;
}

// The interface locale is an explicit choice first (the picker in the masthead
// persists it), then the browser preference, then the catalog default. It used
// to come from `navigator.language` alone, so an operator browsing in en-US had
// no way to reach the complete `es` catalog that already ships.
let LANG = storedLocale() || navigatorLocale();

function t(key) {
  return CATALOG.locales[LANG]?.[key] || FALLBACK[key] || key;
}

// One/other selection without an ICU dependency: `<key>One` is the singular and
// `<key>` the plural; a locale that ships only the base key keeps working.
function pluralKey(key, count) {
  return count === 1 && CATALOG.locales[LANG]?.[`${key}One`] !== undefined ? `${key}One` : key;
}

function countLabel(count, key) {
  return `${count} ${t(pluralKey(key, count))}`;
}

// Corrected before the first paint: the script is deferred, so this runs right
// after parsing and the real locale replaces the catalog default the markup
// ships with, instead of leaving a screen reader with the wrong language while
// the page loads.
document.documentElement.lang = LANG;

const PAGE_SIZE = 10;
const MAX_DEEP_DOCUMENTS = 20;
const MAX_FRAGMENTS = 8;
const MAX_FRAGMENT_BYTES = 512;
const MAX_DOCUMENT_BYTES = 8 * 1024 * 1024;
const MAX_SAVED_PAYLOAD_BYTES = 1024 * 1024;
let searchAvailable = false;
let syntheticSourceMode = false;
// Whether `/status` last reported usable local persistence. When it is false,
// saving a document can only answer 503, so the Save control is disabled up
// front instead of offering a click the server already refuses.
let storageReady = false;

// Setup auth helper for the global api client
globalThis.authHeaders = (extra) => {
  const headers = Object.assign({ Accept: "application/json" }, extra || {});
  if (tokenInput.value) headers.Authorization = `Bearer ${tokenInput.value}`;
  return headers;
};

// UI Elements
const form = document.querySelector("#search-form");
const queryInput = document.querySelector("#query");
const languageInput = document.querySelector("#language");
const regionInput = document.querySelector("#region");
const fileTypeInput = document.querySelector("#file-type");
const tokenInput = document.querySelector("#local-token");
const searchButton = document.querySelector("#search-button");
const searchActions = document.querySelector(".search-actions");
const deepButton = document.querySelector("#deep-button");
const deepMaxFetchesInput = document.querySelector("#deep-max-fetches");
const resultHeading = document.querySelector("#result-heading");
const statusNode = document.querySelector("#status");
const loadingNode = document.querySelector("#loading");
const resultsNode = document.querySelector("#results");
const resultsEmpty = document.querySelector("#results-empty");
const paginationNode = document.querySelector("#pagination");
const previousButton = document.querySelector("#previous");
const nextButton = document.querySelector("#next");
const pageLabel = document.querySelector("#page-label");
const resultTemplate = document.querySelector("#result-template");
const deepTemplate = document.querySelector("#deep-template");
const fragmentTemplate = document.querySelector("#fragment-template");
const cancelButton = document.querySelector("#cancel-button");
const themeToggle = document.querySelector("#theme-toggle");
const uiLanguage = document.querySelector("#ui-language");

// Initialize store and subscribe to changes
store.subscribe((state) => {
  render(state);
});

function resolvedTheme() {
  const explicit = document.documentElement.getAttribute("data-theme");
  if (explicit) return explicit;
  const prefersLight =
    typeof globalThis.matchMedia === "function" &&
    globalThis.matchMedia("(prefers-color-scheme: light)").matches;
  return prefersLight ? "light" : "dark";
}

function updateThemeToggle() {
  const isLight = resolvedTheme() === "light";
  themeToggle.setAttribute("aria-pressed", String(isLight));
  themeToggle.setAttribute("aria-label", isLight ? t("themeToggleToDark") : t("themeToggleToLight"));
  themeToggle.querySelector(".icon-sun").classList.toggle("is-active", !isLight);
  themeToggle.querySelector(".icon-moon").classList.toggle("is-active", isLight);
}

themeToggle.addEventListener("click", () => {
  const next = resolvedTheme() === "light" ? "dark" : "light";
  applyTheme(next);
  try {
    localStorage.setItem(THEME_KEY, next);
  } catch {}
  updateThemeToggle();
});

// Interface language: an explicit, persisted operator choice. Switching
// re-applies every translation in place — no reload, no lost results — and
// re-renders the panels whose labels come from the catalog, while
// `document.documentElement.lang` follows so assistive technology announces the
// new language.
function setLocale(next) {
  const resolved = catalogLocale(next) || CATALOG.defaultLocale;
  if (resolved === LANG) return;
  LANG = resolved;
  try {
    localStorage.setItem(LANGUAGE_KEY, LANG);
  } catch {}
  uiLanguage.value = LANG;
  applyTranslations();
  render(store.get());
  refreshPanels();
  // The status line is catalog-derived as well, so it is re-rendered from its
  // own thunk instead of keeping the previous language until the next action.
  rerenderStatus();
}

uiLanguage.addEventListener("change", () => setLocale(uiLanguage.value));

function safeHttpUrl(value) {
  if (typeof value !== "string" || value.length > 8192) return null;
  try {
    const parsed = new URL(value);
    const allowedScheme = parsed.protocol === "http:" || parsed.protocol === "https:";
    return allowedScheme && !parsed.username && !parsed.password ? parsed : null;
  } catch (_) {
    return null;
  }
}

function boundedText(value, maximum) {
  return typeof value === "string" ? value.slice(0, maximum) : "";
}

function validHash(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function setBusy(busy, mode) {
  searchButton.disabled = busy || !searchAvailable;
  deepButton.disabled = busy;
  searchButton.dataset.active = String(busy && mode === "search");
  deepButton.dataset.active = String(busy && mode === "deep");
  // `aria-busy` belongs on the region that actually changes (`#results`), not on
  // the query input: the input is not what is being replaced. The spinner stays
  // `aria-hidden` and the progress text goes to the `#status` live region.
  resultsNode.setAttribute("aria-busy", String(busy));
  loadingNode.hidden = !busy;
  loadingNode.setAttribute("aria-hidden", String(!busy));
  cancelButton.hidden = !busy;
  cancelButton.setAttribute("aria-hidden", String(!busy));
}

// The status message is composed from the catalog, so it is kept as a render
// function rather than a finished string: switching the interface language can
// then re-render the *same* message in the new language instead of leaving the
// previous one on screen until the next action.
let statusRenderer = null;
let statusState = "";

function renderStatus(render, status) {
  statusRenderer = render;
  statusState = status;
  statusNode.textContent = render();
  statusNode.dataset.state = status;
}

// For messages that are a single catalog key (the common case).
function setStatusKey(key, status) {
  renderStatus(() => t(key), status);
}

// Re-applies the current message after the locale changed.
function rerenderStatus() {
  if (statusRenderer) renderStatus(statusRenderer, statusState);
}

function addOperator(parts, name, value) {
  const clean = value.trim().replace(/[\s"]/g, "");
  if (clean) parts.push(`${name}:${clean}`);
}

function queryText() {
  const parts = [queryInput.value.trim()];
  addOperator(parts, "lang", languageInput.value);
  addOperator(parts, "region", regionInput.value);
  addOperator(parts, "filetype", fileTypeInput.value);
  return parts.join(" ");
}

// ── Filters disclosure ──────────────────────────────────────────
// The panel is collapsed by default, so the summary carries the active state: a
// stale region/type is visible without opening it, and a missing requirement
// opens it (see `reportFailure`) instead of hiding the fix.
const filtersDetails = document.querySelector("details.filters");
const filtersSummary = document.querySelector("#filters-summary");
const accessDetails = document.querySelector("details.access-filters");
const sourceCredentialsDetails = document.querySelector("details.source-credentials");
const accessSummary = document.querySelector("#access-summary");

function updateFiltersSummary() {
  const active = [];
  if (languageInput.value.trim()) {
    active.push(`${t("resultLanguageLabel")}: ${boundedText(languageInput.value.trim(), 16)}`);
  }
  if (regionInput.value.trim()) {
    active.push(`${t("regionLabel")}: ${boundedText(regionInput.value.trim(), 32)}`);
  }
  if (fileTypeInput.value) active.push(`${t("fileTypeLabel")}: ${fileTypeInput.value}`);
  filtersSummary.textContent = active.length
    ? `${t("filtersLabel")} — ${active.join(" · ")}`
    : t("filtersLabel");
}

// The token is a credential, not a query filter: it reports on its own
// disclosure so "is a token set?" is answerable at a glance without implying the
// token narrows the search.
function updateAccessSummary() {
  accessSummary.textContent = tokenInput.value
    ? `${t("accessLabel")} — ${t("tokenLabel")}: ${t("filterValueSet")}`
    : t("accessLabel");
}

for (const field of [languageInput, regionInput, fileTypeInput]) {
  field.addEventListener("input", updateFiltersSummary);
  field.addEventListener("change", updateFiltersSummary);
}
for (const event of ["input", "change"]) {
  tokenInput.addEventListener(event, updateAccessSummary);
}

function searchMetadata(result) {
  const values = [];
  if (typeof result.published_at === "string") values.push(result.published_at.slice(0, 64));
  if (Array.isArray(result.providers) && result.providers.length === 1) {
    values.push(`${t("source")} ${boundedText(result.providers[0], 80)}`);
  }
  return values.join(" · ");
}

function renderSearchResult(result) {
  const url = safeHttpUrl(result.canonical_url);
  if (!url || result.status !== "visible") return;
  const state = store.get();
  const fragment = resultTemplate.content.cloneNode(true);
  const selection = fragment.querySelector(".deep-target-select");
  if (state.mode === "search") {
    selection.querySelector("span").textContent = t("deepTargetSelect");
    const checkbox = selection.querySelector(".deep-target-checkbox");
    const provider = Array.isArray(result.providers) && typeof result.providers[0] === "string"
      ? result.providers[0]
      : "";
    const target = {
      url: url.href,
      title: boundedText(result.title, 512) || null,
      provider,
    };
    checkbox.checked = state.selectedTargets.some((value) => value.url === target.url);
    checkbox.disabled = !provider;
    checkbox.addEventListener("change", () => {
      let selected = store.get().selectedTargets;
      selected = selected.filter((value) => value.url !== target.url);
      if (checkbox.checked) selected.push(target);
      // Transient UI state: update without notifying subscribers so toggling a
      // target neither rebuilds the whole result list nor drops keyboard focus
      // (the store's `render` would `replaceChildren()` the list). The value
      // stays readable via `store.get()` for `run()` (deep targets).
      store.updateSilent({ selectedTargets: selected });
    });
  } else {
    selection.remove();
  }
  const link = fragment.querySelector(".result-title");
  link.href = url.href;
  link.textContent = boundedText(result.title, 300) || boundedText(result.domain, 255) || url.hostname;
  fragment.querySelector(".result-url").textContent = boundedText(result.domain, 255) || url.hostname;
  const snippet = fragment.querySelector(".result-snippet");
  snippet.textContent = boundedText(result.snippet, 2000) || t("noDescription");
  const meta = fragment.querySelector(".result-meta");
  meta.textContent = searchMetadata(result);
  meta.hidden = !meta.textContent;
  resultsNode.append(fragment);
}

function documentStatus(value) {
  if (value === "enriched") return t("enriched");
  if (value === "superficial") return t("superficial");
  return t("unavailable");
}

function acquisitionMethod(value) {
  if (value === "http") return t("http");
  if (value === "rendered") return t("rendered");
  return t("unspecified");
}

function signalLabel(value) {
  if (value === "query_match") return t("queryMatch");
  if (value === "citation") return t("citation");
  if (value === "temporal") return t("temporal");
  if (value === "numeric") return t("numeric");
  return null;
}

function appendSafeUrl(node, value) {
  const url = safeHttpUrl(value);
  if (!url) {
    node.textContent = t("unavailable");
    return;
  }
  const link = document.createElement("a");
  link.href = url.href;
  link.rel = "noopener noreferrer";
  link.textContent = url.href;
  node.append(link);
}

function bytesToHex(bytes) {
  return Array.from(new Uint8Array(bytes), (value) => value.toString(16).padStart(2, "0")).join("");
}

async function verifyFragment(fragment, sourceBytes) {
  const start = fragment.start_byte;
  const end = fragment.end_byte;
  if (!sourceBytes || !Number.isSafeInteger(start) || !Number.isSafeInteger(end)) return "failed";
  if (start < 0 || end <= start || end > sourceBytes.length) return "failed";
  let decoded;
  try {
    decoded = new TextDecoder("utf-8", { fatal: true }).decode(sourceBytes.slice(start, end));
  } catch (_) {
    return "failed";
  }
  if (decoded !== fragment.text) return "failed";
  if (!validHash(fragment.fragment_hash) || !globalThis.crypto?.subtle) return "range_only";
  const digest = await globalThis.crypto.subtle.digest("SHA-256", sourceBytes.slice(start, end));
  return bytesToHex(digest) === fragment.fragment_hash ? "verified" : "failed";
}

function setVerification(node, result) {
  node.dataset.state = result;
  if (result === "verified") node.textContent = t("verified");
  else if (result === "range_only") node.textContent = t("rangeOnly");
  else node.textContent = t("notVerifiable");
}

function evidenceFragments(evidence) {
  const provenanceId = evidence?.provenance?.provenance_id;
  if (!validHash(provenanceId) || !Array.isArray(evidence.fragments)) return [];
  return evidence.fragments.slice(0, MAX_FRAGMENTS).filter((fragment) => {
    if (!fragment || fragment.provenance_id !== provenanceId) return false;
    if (typeof fragment.text !== "string" || !fragment.text || fragment.text.length > 2048) return false;
    return new TextEncoder().encode(fragment.text).length <= MAX_FRAGMENT_BYTES;
  });
}

function renderEvidenceFragment(fragment, sourceBytes, list) {
  const item = fragmentTemplate.content.cloneNode(true);
  item.querySelector("blockquote").textContent = fragment.text;
  const signals = item.querySelector(".fragment-signals");
  signals.setAttribute("aria-label", t("observedSignals"));
  const observed = Array.isArray(fragment.signals) ? fragment.signals.slice(0, 8) : [];
  for (const signal of observed) {
    const label = signalLabel(signal);
    if (!label) continue;
    const chip = document.createElement("span");
    chip.textContent = label;
    signals.append(chip);
  }
  if (!signals.childElementCount) signals.hidden = true;
  item.querySelector(".fragment-hash span").textContent = t("sha256");
  const hash = item.querySelector(".fragment-hash code");
  hash.textContent = validHash(fragment.fragment_hash) ? fragment.fragment_hash : t("unavailable");
  const verification = item.querySelector(".fragment-verification");
  verification.textContent = t("verifying");
  list.append(item);
  verifyFragment(fragment, sourceBytes)
    .then((result) => {
      if (verification.isConnected) setVerification(verification, result);
    })
    .catch(() => {
      if (verification.isConnected) setVerification(verification, "failed");
    });
}

function deepDocumentMetadata(documentValue) {
  const values = [documentStatus(documentValue.status)];
  if (typeof documentValue.content_type === "string") {
    values.push(documentValue.content_type.slice(0, 120));
  }
  if (Number.isSafeInteger(documentValue.size) && documentValue.size >= 0) {
    values.push(`${documentValue.size.toLocaleString(LANG)} ${t("bytes")}`);
  }
  return values.join(" · ");
}

function renderDeepDocument(item) {
  const documentValue = item.document;
  const evidence = item.evidence;
  const url = safeHttpUrl(documentValue.final_url) || safeHttpUrl(documentValue.canonical_url);
  const fragment = deepTemplate.content.cloneNode(true);
  const link = fragment.querySelector(".result-title");
  if (url) {
    link.href = url.href;
    link.textContent = boundedText(documentValue.title, 300) || url.hostname;
    fragment.querySelector(".result-url").textContent = url.hostname;
  } else {
    link.removeAttribute("href");
    link.textContent = boundedText(documentValue.title, 300) || t("noUrl");
    fragment.querySelector(".result-url").textContent = t("noOrigin");
  }
  const status = fragment.querySelector(".document-status");
  status.textContent = documentStatus(documentValue.status);
  status.dataset.state = documentValue.status === "enriched" ? "success" : "partial";
  fragment.querySelector(".document-meta").textContent = deepDocumentMetadata(documentValue);
  fragment.querySelector(".evidence-panel").setAttribute("aria-label", t("evidenceFragmentsHeading"));
  fragment.querySelector(".evidence-heading h4").textContent = t("evidenceFragmentsHeading");
  fragment.querySelector(".provenance summary").textContent = t("provenanceHeading");
  const provenanceLabels = {
    ".provenance-original": "originalUrl",
    ".provenance-canonical": "canonicalUrl",
    ".provenance-final": "finalUrl",
    ".provenance-method": "acquisition",
    ".provenance-extractor": "extractor",
    ".provenance-retrieved": "retrieved",
    ".provenance-source-hash": "sourceHash",
    ".provenance-content-hash": "extractedTextHash",
  };
  for (const [selector, key] of Object.entries(provenanceLabels)) {
    const value = fragment.querySelector(selector);
    const label = value?.closest("div")?.querySelector("dt");
    if (label) label.textContent = t(key);
  }

  const list = fragment.querySelector(".evidence-fragments");
  const empty = fragment.querySelector(".empty-evidence");
  const fragments = evidenceFragments(evidence);
  const provenance = evidence && evidence.provenance && typeof evidence.provenance === "object"
    ? evidence.provenance
    : {};
  let sourceBytes = null;
  if (typeof documentValue.content === "string" && documentValue.content.length <= MAX_DOCUMENT_BYTES) {
    const encoded = new TextEncoder().encode(documentValue.content);
    if (encoded.length <= MAX_DOCUMENT_BYTES) sourceBytes = encoded;
  }
  for (const evidenceFragment of fragments) {
    renderEvidenceFragment(evidenceFragment, sourceBytes, list);
  }
  const renderedCount = list.childElementCount;
  fragment.querySelector(".fragment-count").textContent = `${renderedCount} ${t("fragmentCount")}`;
  if (!renderedCount) {
    empty.hidden = false;
    empty.textContent = documentValue.status === "superficial"
      ? t("superficialDocument")
      : t("noFragments");
  }

  appendSafeUrl(fragment.querySelector(".provenance-original"), provenance.original_url);
  appendSafeUrl(fragment.querySelector(".provenance-canonical"), provenance.canonical_url);
  appendSafeUrl(fragment.querySelector(".provenance-final"), provenance.final_url);
  fragment.querySelector(".provenance-method").textContent = acquisitionMethod(provenance.fetch_method);
  fragment.querySelector(".provenance-extractor").textContent = boundedText(provenance.extractor_used, 160) || t("noExtractor");
  fragment.querySelector(".provenance-retrieved").textContent = boundedText(provenance.retrieved_at, 64) || t("unavailable");
  fragment.querySelector(".provenance-source-hash").textContent = validHash(provenance.source_content_hash)
    ? provenance.source_content_hash
    : t("unavailable");
  fragment.querySelector(".provenance-content-hash").textContent = validHash(provenance.extracted_content_hash)
    ? provenance.extracted_content_hash
    : t("unavailable");

  const saveButton = fragment.querySelector(".save-document");
  saveButton.textContent = t("savedAction");
  // With persistence off (or unknown) the save can only 503, and the reason is
  // already on screen in the service panel, so the control is disabled instead
  // of offering a click that cannot succeed.
  saveButton.disabled = !storageReady;
  saveButton.title = storageReady ? "" : t("savedFailedStorage");
  saveButton.addEventListener("click", () => saveDocument(documentValue, provenance, saveButton));

  resultsNode.append(fragment);
}

function render(state) {
  // While a request is in flight the committed view stays on screen: results
  // are only replaced once a payload has been accepted, so a failed, cancelled
  // or raced request can no longer destroy work the operator still needs.
  if (state.pending) return;
  resultsNode.replaceChildren();
  resultsEmpty.hidden = state.mode !== "search" || state.items.length > 0;
  if (state.mode === "deep") {
    for (const item of state.items) renderDeepDocument(item);
    paginationNode.hidden = true;
    return;
  }
  for (const result of state.items) renderSearchResult(result);
  // With an unknown total, show the paginator whenever a further page may
  // exist; with a known total, only when there is more than one page.
  paginationNode.hidden = state.knownTotal
    ? state.totalPages <= 1
    : !state.hasMore && state.page === 0;
  pageLabel.textContent = state.knownTotal
    ? `${t("pageOf")} ${state.page + 1} ${t("of")} ${state.totalPages}`
    : `${t("pageOf")} ${state.page + 1}`;
  previousButton.disabled = state.page === 0;
  nextButton.disabled = state.knownTotal
    ? state.page + 1 >= state.totalPages
    : !state.hasMore;
}

function deepItems(payload) {
  const evidenceByDocument = new Map();
  for (const evidence of payload.evidence_v2.slice(0, MAX_DEEP_DOCUMENTS)) {
    if (!evidence || evidence.evidence_version !== "v2" || typeof evidence.document_id !== "string") continue;
    const provenance = evidence.provenance;
    if (!provenance || provenance.document_id !== evidence.document_id || !validHash(provenance.provenance_id)) continue;
    if (!evidenceByDocument.has(evidence.document_id)) evidenceByDocument.set(evidence.document_id, evidence);
  }
  return payload.documents
    .filter((documentValue) => documentValue && typeof documentValue === "object")
    .slice(0, MAX_DEEP_DOCUMENTS)
    .map((documentValue) => {
      const candidate = evidenceByDocument.get(documentValue.search_result_id) || null;
      const provenance = candidate?.provenance;
      const lineageMatches = provenance
        && provenance.original_url === documentValue.original_url
        && provenance.canonical_url === documentValue.canonical_url
        && provenance.final_url === documentValue.final_url;
      return { document: documentValue, evidence: lineageMatches ? candidate : null };
    });
}

function validatePayload(payload, mode) {
  if (!payload || payload.schema_version !== "1") return false;
  if (mode === "deep") {
    return Array.isArray(payload.documents) && Array.isArray(payload.evidence_v2);
  }
  return Array.isArray(payload.results);
}

// A credential failure is only actionable when the field that fixes it is
// reachable: the token lives inside its own collapsed "Server access" panel,
// so open that one before moving focus there instead of leaving the operator
// to hunt for it.
function focusTokenField() {
  accessDetails.open = true;
  tokenInput.focus();
}

function reportFailure(error) {
  if (error.name === "AbortError") return;
  const key = ERROR_MESSAGE_KEYS[error.code];
  if (key) {
    setStatusKey(key, "error");
    if (error.code === "unauthorized" || error.code === "scope_denied") focusTokenField();
  } else if (error.status === 401 || error.message.includes("401")) {
    setStatusKey("unauthorized", "error");
    focusTokenField();
  } else if (error.status === 429 || error.message.includes("429")) {
    setStatusKey("rateLimited", "error");
  } else if (error.status === 504 || error.message.includes("504")) {
    setStatusKey("timeout", "error");
  } else {
    setStatusKey("genericError", "error");
  }
}

const ERROR_MESSAGE_KEYS = {
  invalid_request: "errorInvalidRequest",
  invalid_query: "errorInvalidQuery",
  unauthorized: "unauthorized",
  rate_limited: "rateLimited",
  request_timeout: "timeout",
  egress_denied: "errorEgressDenied",
  provider_not_declared: "errorProviderUnavailable",
  provider_not_registered: "errorProviderUnavailable",
  provider_not_enabled: "errorProviderUnavailable",
  provider_not_approved: "errorProviderUnavailable",
  provider_credential_missing: "errorProviderUnavailable",
  provider_network_blocked: "errorProviderUnavailable",
  search_planning_failed: "errorSearchUnavailable",
  inference_unavailable: "errorSearchUnavailable",
  ranking_backend_unavailable: "errorSearchUnavailable",
  storage_unavailable: "errorStorageUnavailable",
  scope_denied: "scopeDenied",
};

async function run(mode, page) {
  const state = store.get();
  if (state.controller) state.controller.abort();
  const controller = new AbortController();
  const previousPage = state.page;

  // The committed view is not reset here: `pending` suppresses the subscriber
  // render, so the previous results stay on screen until a payload is accepted
  // and a failed, cancelled or raced request leaves the last valid set intact
  // instead of blanking the page (it used to `replaceChildren()` before the
  // fetch, destroying work the operator could still use).
  store.update({ controller, pending: true, page: mode === "deep" ? 0 : page });

  const selectedTargets = mode === "deep" ? store.get().selectedTargets.slice() : [];
  // A new search rebuilds the result set, so a Deep selection only stays valid if
  // its URL is still present. The selection is reconciled against the payload
  // once it commits (see below) instead of being dropped here: refining a query
  // then re-searching used to silently lose every checkbox the operator had set.

  resultHeading.textContent = mode === "deep"
    ? t("evidenceHeading")
    : t("resultsHeading");
  setBusy(true, mode);
  setStatusKey(
    mode === "deep" ? "analyzing" : "searching",
    "loading",
  );
  try {
    const endpoint = mode === "deep" ? "/deep" : "/search";
    const body = { q: queryText() };
    if (mode === "deep") {
      if (selectedTargets.length) body.targets = selectedTargets;
      if (deepMaxFetchesInput.value) body.max_fetches = Number(deepMaxFetchesInput.value);
    }
    if (mode === "search") {
      body.page = store.get().page;
      body.page_size = PAGE_SIZE;
    }
    
    // Use centralized API client
    const payload = await globalThis.api.post(endpoint, body, {
      signal: controller.signal,
    });
    
    // Verify controller hasn't changed
    if (store.get().controller !== controller) return;
    if (!validatePayload(payload, mode)) throw new Error("invalid_contract");
    
    if (mode === "deep") {
      const items = deepItems(payload);
      store.update({ pending: false, mode, page: 0, items });
      const fragments = items.reduce(
        (total, item) => total + evidenceFragments(item.evidence).length,
        0,
      );
      if (!items.length) {
        setStatusKey("noEvidence", "partial");
      } else if (!fragments || (Array.isArray(payload.degradations) && payload.degradations.length)) {
        renderStatus(() => countLabel(items.length, "partialEvidence"), "partial");
      } else {
        renderStatus(
          () => `${countLabel(items.length, "evidenceCount")} ${fragments} ${t("evidenceFragments")}`,
          "success",
        );
      }
    } else {
      const items = payload.results;
      const pageSize = typeof payload.page_size === "number" && payload.page_size > 0
        ? payload.page_size
        : PAGE_SIZE;
      // A native provider page omits `total_results`; the page length is then
      // not a whole-set size. Track both the known-total case and, when the
      // total is unknown, whether the page came back full (so a next page may
      // exist) instead of collapsing to a single page.
      const knownTotal = typeof payload.total_results === "number";
      const totalResults = knownTotal ? payload.total_results : items.length;
      const hasMore = knownTotal
        ? false
        : items.length >= pageSize;

      const currentPage = typeof payload.page === "number" ? payload.page : store.get().page;

      // Keep only the Deep selections whose canonical URL survived the new result
      // set: the checkboxes the operator already ticked stay ticked when the
      // result is still there, and a stale URL is dropped instead of pointing at
      // something no longer on screen.
      const visibleUrls = new Set(
        items
          .map((result) => safeHttpUrl(result.canonical_url)?.href)
          .filter((href) => typeof href === "string"),
      );
      const survivingTargets = store
        .get()
        .selectedTargets.filter((target) => visibleUrls.has(target.url));

      store.update({
        pending: false,
        mode,
        items,
        totalResults,
        knownTotal,
        hasMore,
        page: currentPage,
        selectedTargets: survivingTargets,
        totalPages: knownTotal
          ? Math.max(1, Math.ceil(totalResults / pageSize))
          : currentPage + (hasMore ? 2 : 1)
      });
      
      if (!items.length) {
        // An empty set is not a degradation: the amber `partial` tone means
        // partial or degraded answers, so "no results" gets its own neutral
        // state and the note under the list carries the next step.
        setStatusKey("noResults", "empty");
      } else if (payload.status === "partial_success") {
        renderStatus(() => countLabel(items.length, "partialResults"), "partial");
      } else if (!knownTotal) {
        // The whole-set size is unknown: report this page only.
        renderStatus(() => countLabel(items.length, "resultsCount"), "success");
      } else {
        renderStatus(
          () => `${items.length} ${t("of")} ${totalResults} ${t(pluralKey("resultsCount", totalResults))}`,
          "success",
        );
      }
      syncLocation(currentPage);
    }
    // A keyboard operator who submitted from the query field stays there for the
    // next refinement — `#status` already announces the outcome — so focus only
    // moves into the results when the operator was somewhere else.
    if (document.activeElement !== queryInput) resultsNode.focus({ preventScroll: true });
    refreshPanels();
  } catch (error) {
    // Nothing was committed: drop `pending` silently (no subscriber render, so
    // the DOM and any focus inside it survive), restore the page the operator
    // was on, and report the reason.
    store.updateSilent({ pending: false, page: previousPage });
    reportFailure(error);
  } finally {
    if (store.get().controller === controller) {
      store.update({ controller: null });
      setBusy(false, mode);
    }
  }
}

// ── Location state ──────────────────────────────────────────────
// The last committed search lives in the URL, so a refresh or a shared link
// reproduces it instead of losing everything. `replaceState` (not `pushState`)
// keeps one history entry per search: Back still leaves the page, which is what
// a single-view app should do.
function searchLocation(page) {
  const parts = [`q=${encodeURIComponent(queryInput.value.trim())}`];
  if (languageInput.value.trim()) {
    parts.push(`lang=${encodeURIComponent(languageInput.value.trim())}`);
  }
  if (regionInput.value.trim()) {
    parts.push(`region=${encodeURIComponent(regionInput.value.trim())}`);
  }
  if (fileTypeInput.value) parts.push(`filetype=${encodeURIComponent(fileTypeInput.value)}`);
  if (page > 0) parts.push(`page=${page + 1}`);
  return `${location.pathname}?${parts.join("&")}`;
}

function syncLocation(page) {
  try {
    history.replaceState(null, "", searchLocation(page));
  } catch (_) {
    // A blocked History API is not worth interrupting a successful search for.
  }
}

function restoreFromLocation() {
  const pairs = location.search.replace(/^\?/, "").split("&").filter(Boolean);
  const params = new Map();
  for (const pair of pairs) {
    const [key, value = ""] = pair.split("=");
    // `decodeURIComponent` throws `URIError` on a malformed escape (e.g. `%`).
    // A hand-edited or truncated shared link must not abort the bootstrap, so a
    // pair that cannot be decoded is skipped rather than propagated.
    let decodedKey;
    let decodedValue;
    try {
      decodedKey = decodeURIComponent(key);
      decodedValue = decodeURIComponent(value.replace(/\+/g, " "));
    } catch (_) {
      continue;
    }
    params.set(decodedKey, decodedValue);
  }
  const query = boundedText((params.get("q") || "").trim(), 2048);
  if (!query) return;
  queryInput.value = query;
  languageInput.value = boundedText(params.get("lang") || "", 16);
  regionInput.value = boundedText(params.get("region") || "", 32);
  const fileType = params.get("filetype") || "";
  if (["pdf", "html", "txt"].includes(fileType)) fileTypeInput.value = fileType;
  const requested = Number.parseInt(params.get("page") || "1", 10);
  const page = Number.isSafeInteger(requested) && requested > 1 ? requested - 1 : 0;
  updateFiltersSummary();
  run("search", page);
}

// The form is `novalidate`: validation runs here so the message is the app's own
// copy in the operator's locale (native bubbles speak the browser's language
// and sit outside the catalog), the offending field is marked, and the collapsed
// filters panel opens when the fix lives inside it.
function queryValidationError() {
  if (!queryInput.value.trim()) return "errorQueryRequired";
  if (tokenInput.value && tokenInput.value.length < 32) return "errorTokenTooShort";
  return null;
}

function reportValidationError(key) {
  const field = key === "errorTokenTooShort" ? tokenInput : queryInput;
  setStatusKey(key, "error");
  field.setAttribute("aria-invalid", "true");
  // The message lives in the `#status` live region, so associate it with the
  // offending field: a screen reader must hear *why* the field is invalid, not
  // only that it is. `tokenInput` keeps its own helper text in the list.
  field.setAttribute(
    "aria-describedby",
    field === tokenInput ? "token-help status" : "status",
  );
  // Open the disclosure that actually holds the offending field: the token's
  // panel and the filter panel are separate, so a fixed `filtersDetails.open`
  // would leave a token error focused inside a still-collapsed panel.
  if (filtersDetails.contains(field)) filtersDetails.open = true;
  const evidenceDetails = document.querySelector("details.evidence-options");
  if (evidenceDetails.contains(field)) evidenceDetails.open = true;
  if (accessDetails.contains(field)) accessDetails.open = true;
  field.focus();
}

function clearValidationError() {
  if (queryInput.hasAttribute("aria-invalid")) {
    queryInput.removeAttribute("aria-invalid");
    queryInput.removeAttribute("aria-describedby");
  }
  if (tokenInput.hasAttribute("aria-invalid")) {
    tokenInput.removeAttribute("aria-invalid");
    // Restore the token field's own helper binding (its `aria-describedby` in
    // the markup is `token-help`).
    tokenInput.setAttribute("aria-describedby", "token-help");
  }
}

queryInput.addEventListener("input", clearValidationError);
tokenInput.addEventListener("input", clearValidationError);

function execute(event) {
  event.preventDefault();
  const invalid = queryValidationError();
  if (invalid) {
    reportValidationError(invalid);
    return;
  }
  const requested = event.submitter?.dataset.mode;
  const mode = requested === "deep" ? requested : "search";
  run(mode, 0);
}

form.addEventListener("submit", execute);
cancelButton.addEventListener("click", () => {
  const state = store.get();
  if (state.controller) {
    state.controller.abort();
    setStatusKey("cancelled", "partial");
  }
});
window.addEventListener("pagehide", () => {
  const state = store.get();
  if (state.controller) state.controller.abort();
  tokenInput.value = "";
});
previousButton.addEventListener("click", () => {
  const state = store.get();
  if (state.page > 0) run("search", state.page - 1);
});
nextButton.addEventListener("click", () => {
  const state = store.get();
  const canAdvance = state.knownTotal
    ? state.page + 1 < state.totalPages
    : state.hasMore;
  if (canAdvance) run("search", state.page + 1);
});

// ── Source availability ─────────────────────────────────────────
// The source-availability and service-state panels were removed from the UI;
// the operator-tools surface is gone. `fetchProviderStatus` is kept only for
// its side effect: it drives the Marginalia credential row in the search panel
// from the authoritative `/providers` registry.
async function fetchProviderStatus() {
  try {
    // Synthetic search fixtures are not the configured provider registry.
    if (syntheticSourceMode) {
      syncCredentialActions([]);
      return;
    }
    // Use centralized client
    const payload = await globalThis.api.get("/providers");
    if (!payload || !Array.isArray(payload.providers)) return;
    const enabledList = [];
    for (const provider of payload.providers) {
      if (provider.enabled) enabledList.push(provider.name);
    }
    // The credential row follows `providers.enabled`: Marginalia is the only
    // shipped source and always takes a key, so an enabled Marginalia reveals
    // its key controls.
    syncCredentialActions(enabledList);
  } catch (_) {
    // Without provider state (e.g. no token yet) the credential row must not
    // claim any source is ready to take a key: hide it until we have an
    // authoritative answer from `/providers`.
    syncCredentialActions([]);
  }
}

// ── Service state ───────────────────────────────────────────────
// The service-state panel was removed; `/status` is still the authoritative
// source for the search flow's own state (whether search is usable, whether
// evidence persistence is healthy, whether authentication is required).
async function fetchServiceState() {
  try {
    const payload = await globalThis.api.get("/status");
    if (!payload || payload.schema_version !== "1") {
      return null;
    }
    const sources = Array.isArray(payload.sources) ? payload.sources : [];
    const availableSources = sources.filter((source) => source.status === "available").length;
    // Mock mode: `/status` reports the synthetic sources search actually uses
    // (`mock-a`/`mock-b`), so the environment can be labelled instead of letting
    // an evaluator mistake fixtures for real data. A deployment that names its
    // own providers `mock-*` would be mislabelled; that is the price of the
    // payload carrying no dedicated flag.
    const mockOnly = sources.length > 0
      && sources.every((source) => /^mock-[a-z0-9]+$/.test(String(source.name || "")));
    syntheticSourceMode = mockOnly;
    sourceCredentialsDetails.hidden = mockOnly;
    if (mockOnly) syncCredentialActions([]);
    const authenticationRequired = payload.authentication_required !== false;
    accessDetails.hidden = !authenticationRequired;
    if (!authenticationRequired && tokenInput.value) {
      tokenInput.value = "";
      updateAccessSummary();
    }
    searchAvailable = availableSources > 0;
    searchButton.disabled = !searchAvailable;
    searchButton.setAttribute("aria-disabled", String(!searchAvailable));
    searchButton.title = searchAvailable ? "" : t("searchUnavailable");
    if (!searchAvailable) setStatusKey("errorSearchUnavailable", "error");
    const storage = payload.storage && typeof payload.storage === "object" ? payload.storage : {};
    // Only a healthy persistence turns saving on; anything else (off, broken,
    // unreachable) means `/saved` answers 503, so the Save control is disabled.
    storageReady = Boolean(storage.enabled && storage.available);
    syncMarginaliaFromStatus(payload);
    return payload;
  } catch (_) {
    syntheticSourceMode = false;
    // No authoritative `/status` means no evidence persistence is usable: fail
    // closed so a Save click is not offered against an unknown backend.
    storageReady = false;
    sourceCredentialsDetails.hidden = false;
    accessDetails.hidden = false;
    searchAvailable = false;
    searchButton.disabled = true;
    searchButton.setAttribute("aria-disabled", "true");
    searchButton.title = t("searchUnavailable");
    return null;
  }
}

// ── Marginalia API key (option A) ───────────────────────────────
// Operator credential management: a key is inserted into the process's RAM
// (never disk, config, or logs) after a live canary validates it, or liquidated
// on demand. The row sits in its own secondary line so the search axis
// (write → search → analyze) stays visually dominant.
const marginaliaKeyButton = document.querySelector("#marginalia-key-button");
const marginaliaClearButton = document.querySelector("#marginalia-clear-button");
const marginaliaStatus = document.querySelector("#marginalia-status");
const marginaliaDialog = document.querySelector("#marginalia-dialog");
const marginaliaForm = document.querySelector("#marginalia-form");
const marginaliaKeyInput = document.querySelector("#marginalia-key");
const marginaliaTtlInput = document.querySelector("#marginalia-ttl");
const marginaliaKeyError = document.querySelector("#marginalia-key-error");
const marginaliaSave = document.querySelector("#marginalia-save");
const marginaliaCancel = document.querySelector("#marginalia-cancel");
const credentialActions = document.querySelector("#credential-actions");
const marginaliaCredentialGroup = document.querySelector("#marginalia-credential-group");

// Show the Marginalia credential controls only while Marginalia is one of the
// selected sources. The visibility follows `providers.enabled` — the same
// single source of truth the source panel drives — so the key
// area can never drift from the active selection. Marginalia is the only
// credential-gated source, so it is the only one with a key row.
function syncCredentialActions(enabledList) {
  const list = Array.isArray(enabledList) ? enabledList : [];
  const marginaliaOn = list.includes("marginalia");
  if (marginaliaCredentialGroup) marginaliaCredentialGroup.hidden = !marginaliaOn;
  if (credentialActions) credentialActions.hidden = !marginaliaOn;
}

function marginaliaCredentialSource(sources) {
  const entry = (Array.isArray(sources) ? sources : []).find(
    (source) => source && source.name === "marginalia",
  );
  return entry && typeof entry.credential_source === "string" ? entry.credential_source : "none";
}

// Human, coarse expiry label from a remaining-milliseconds value. Rounds up to
// the next unit so a key never reads as "0 min" while still usable, and returns
// null for the "never expires" sentinel the server omits.
function formatRemaining(ms) {
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const minutes = Math.ceil(ms / 60000);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.ceil(minutes / 60);
  return `${hours} h`;
}

function updateMarginaliaUi(source, expiresInMs) {
  // Only an in-memory key can be liquidated from here: a key read from the
  // environment variable is out of this surface's reach, so "liquidate" stays
  // disabled and the status says where the key actually comes from.
  marginaliaClearButton.disabled = source !== "ephemeral";
  marginaliaStatus.hidden = false;
  if (source === "ephemeral") {
    marginaliaStatus.dataset.state = "success";
    const remaining = formatRemaining(Number(expiresInMs));
    marginaliaStatus.textContent = remaining
      ? `${t("marginaliaStatusPresent")} · ${t("marginaliaExpiresIn")} ${remaining}`
      : t("marginaliaStatusPresent");
  } else if (source === "environment") {
    marginaliaStatus.dataset.state = "partial";
    marginaliaStatus.textContent = t("marginaliaStatusEnvironment");
  } else {
    marginaliaStatus.dataset.state = "partial";
    marginaliaStatus.textContent = t("marginaliaStatusAbsent");
  }
}

// Keep the two buttons in sync with the last `/status` payload.
function syncMarginaliaFromStatus(payload) {
  if (!payload) return;
  updateMarginaliaUi(marginaliaCredentialSource(payload.sources));
}

function openMarginaliaDialog() {
  marginaliaKeyInput.value = "";
  marginaliaKeyInput.removeAttribute("aria-invalid");
  marginaliaKeyError.hidden = true;
  marginaliaDialog.showModal();
  marginaliaKeyInput.focus();
}

marginaliaKeyButton.addEventListener("click", openMarginaliaDialog);
marginaliaCancel.addEventListener("click", () => marginaliaDialog.close());
// Clicking the backdrop (outside the dialog box) closes it, like a native modal.
marginaliaDialog.addEventListener("click", (event) => {
  if (event.target === marginaliaDialog) marginaliaDialog.close();
});

async function saveMarginaliaKey(event) {
  event.preventDefault();
  const key = marginaliaKeyInput.value.trim();
  if (!key) {
    marginaliaKeyError.hidden = false;
    marginaliaKeyError.textContent = t("marginaliaEmpty");
    marginaliaKeyInput.setAttribute("aria-invalid", "true");
    return;
  }
  marginaliaSave.disabled = true;
  marginaliaKeyError.hidden = true;
  try {
    const ttlMs = Number(marginaliaTtlInput && marginaliaTtlInput.value);
    const body = { key };
    if (Number.isFinite(ttlMs) && ttlMs >= 0) body.ttl_ms = ttlMs;
    const report = await globalThis.api.post("/providers/marginalia/credential", body);
    marginaliaDialog.close();
    marginaliaKeyInput.value = "";
    updateMarginaliaUi(
      report && report.available ? "ephemeral" : "none",
      report && report.expires_in_ms,
    );
    setStatusKey("marginaliaSaved", "success");
    fetchServiceState();
  } catch (error) {
    // 401/403 means the operator token lacks admin scope; anything else means
    // the key itself did not pass the live canary (or the source was
    // unreachable) and nothing was stored. Classification reads the structured
    // `error.status`/`error.code` the shared client sets — real error bodies
    // carry a catalog message with no status digits — and the message check
    // survives only as a fallback for non-JSON error pages.
    const status = Number(error && error.status) || 0;
    const code = error && typeof error.code === "string" ? error.code : "";
    const message = String(error && error.message);
    marginaliaKeyError.hidden = false;
    marginaliaKeyInput.setAttribute("aria-invalid", "true");
    // 401 = no valid token was sent (nothing to do with the key); 403 = the
    // token lacks admin scope; anything else = the key failed the live canary.
    marginaliaKeyError.textContent =
      status === 401 || code === "unauthorized" || message.includes("401")
        ? t("unauthorized")
        : status === 403 || code === "scope_denied" || message.includes("403")
          ? t("marginaliaFailed")
          : t("marginaliaInvalid");
  } finally {
    marginaliaSave.disabled = false;
  }
}

marginaliaForm.addEventListener("submit", saveMarginaliaKey);

marginaliaClearButton.addEventListener("click", async () => {
  try {
    await globalThis.api.delete("/providers/marginalia/credential");
    updateMarginaliaUi("none");
    setStatusKey("marginaliaCleared", "success");
    fetchServiceState();
  } catch (_) {
    setStatusKey("marginaliaClearedFailed", "error");
  }
});

// ── History ─────────────────────────────────────────────────────
// The search-history panel was removed; queries are no longer listed or
// purged from this surface.

// ── Saved documents ─────────────────────────────────────────────
// The saved-documents list was removed. Saving one Deep document from the
// results view still works; the collection is no longer enumerated here.

// ── Saved document creation ─────────────────────────────────────
// Persists one Deep document as a bounded whitelist of small fields. The
// payload deliberately never carries the document's extracted `content` (or any
// other bulk field): the UI posts provenance and metadata only, so saved
// documents stay small and never duplicate the full extraction.
// A save can fail for reasons the operator can act on (storage off on the
// server, a token without write scope) or for reasons they cannot. Collapsing
// every failure into one generic line — while leaving the button looking ready
// for another click — is what made this a High finding: the reason is now
// specific, it is shown next to the control that failed, and the document stays
// on screen so nothing the operator produced is lost.
function saveFailureKey(error) {
  const code = typeof error?.code === "string" ? error.code : "";
  if (code === "storage_unavailable" || error?.status === 503) return "savedFailedStorage";
  if (
    code === "unauthorized"
    || code === "scope_denied"
    || error?.status === 401
    || error?.status === 403
  ) {
    return "savedFailedUnauthorized";
  }
  return "savedFailed";
}

// Per-surface status line: `#status` is the global live region and whatever ran
// last overwrites it, so a message about one card belongs next to that card. The
// key is kept on the node (like `announce`) so a locale switch re-renders it.
function setInlineStatus(node, key, state) {
  if (!node) return;
  if (key) {
    node.dataset.i18nKey = key;
    node.textContent = t(key);
  } else {
    node.textContent = "";
    delete node.dataset.i18nKey;
  }
  node.hidden = !key;
  if (state) node.dataset.state = state;
  else node.removeAttribute("data-state");
}

async function saveDocument(documentValue, provenance, button) {
  const inline = button.closest(".document-actions")?.querySelector(".document-save-status") || null;
  const url = safeHttpUrl(documentValue.canonical_url);
  if (!url || !validHash(documentValue.content_hash)) {
    setInlineStatus(inline, "savedFailed", "error");
    setStatusKey("savedFailed", "error");
    return;
  }
  const payload = JSON.stringify({
    canonical_url: url.href,
    final_url: boundedText(documentValue.final_url, 8192),
    title: boundedText(documentValue.title, 300),
    status: boundedText(documentValue.status, 32),
    content_hash: documentValue.content_hash,
    retrieved_at: boundedText(provenance.retrieved_at, 64),
  });
  if (new TextEncoder().encode(payload).length > MAX_SAVED_PAYLOAD_BYTES) {
    setInlineStatus(inline, "savedFailed", "error");
    setStatusKey("savedFailed", "error");
    return;
  }
  button.disabled = true;
  setInlineStatus(inline, "", null);
  try {
    await globalThis.api.post("/saved", {
      canonical_url: url.href,
      title: boundedText(documentValue.title, 300),
      snippet: boundedText(documentValue.excerpt, 2000),
      content_hash: documentValue.content_hash,
      extractor_version: boundedText(provenance.extractor_used, 128) || "unknown",
      payload,
      source_query: boundedText(queryInput.value, 2048),
      tags: [],
    });
    setStatusKey("savedDone", "success");
    button.textContent = t("savedActionDone");
    setInlineStatus(inline, "savedDone", "success");
  } catch (error) {
    // The button stays usable and now says what it does: the operator can save
    // again the moment storage is back, and the reason sits next to the editor
    // instead of only in the global status line.
    const key = saveFailureKey(error);
    button.disabled = false;
    button.textContent = t("savedAgain");
    setInlineStatus(inline, key, "error");
    setStatusKey(key, "error");
  }
}

// ── Refresh ─────────────────────────────────────────────────────
// `/status` is the authoritative source for the search flow's own state
// (search usability, evidence persistence, authentication) and for the
// Marginalia credential row. It is refreshed whenever the operator token
// changes, so a credential entered later takes effect without a reload.
async function refreshPanels() {
  await fetchServiceState();
  fetchProviderStatus();
}

tokenInput.addEventListener("change", refreshPanels);
refreshPanels();

function applyDeclarativeTranslations() {
  for (const node of document.querySelectorAll("[data-i18n]")) {
    node.textContent = t(node.dataset.i18n);
  }
  for (const node of document.querySelectorAll("[data-i18n-placeholder]")) {
    node.placeholder = t(node.dataset.i18nPlaceholder);
  }
  for (const node of document.querySelectorAll("[data-i18n-aria-label]")) {
    node.setAttribute("aria-label", t(node.dataset.i18nAriaLabel));
  }
  // Per-surface messages announced through `announce()` keep the key they were
  // rendered from, so they follow a locale switch like the static copy does.
  for (const node of document.querySelectorAll("[data-i18n-key]")) {
    node.textContent = t(node.dataset.i18nKey);
  }
}

// Every imperative translation of static markup lives in this one function so
// the locale can be switched at runtime (the masthead picker) without a reload;
// the `data-i18n*` attributes cover the rest, and the composed strings (filters
// summary, empty-state note) are refreshed in the same pass.
function applyTranslations() {
  document.documentElement.lang = LANG;
  applyDeclarativeTranslations();
  document.title = t("pageTitle");
  resultsEmpty.textContent = t("resultsEmptyHint");
  document.querySelector("#search-heading").textContent = t("searchHeading");
  document.querySelector("#search-button").textContent = t("searchButton");
  document.querySelector("#deep-button").textContent = t("deepButton");
  document.querySelector("label[for='deep-max-fetches']").textContent = t("deepMaxFetchesLabel");
  deepMaxFetchesInput.placeholder = t("deepMaxFetchesPlaceholder");
  document.querySelector("#previous").textContent = t("previous");
  document.querySelector("#next").textContent = t("next");
  document.querySelector("#cancel-button").textContent = t("cancel");
  document.querySelector(".skip-link").textContent = t("skipToResults");
  document.querySelector(".brand").setAttribute("aria-label", t("brandLabel"));
  document.querySelector("#result-heading").textContent = t("resultsHeading");
  queryInput.placeholder = t("searchPlaceholder");
  document.querySelector("#filters-summary").textContent = t("filtersLabel");
  document.querySelector("label[for='language']").textContent = t("resultLanguageLabel");
  document.querySelector("label[for='region']").textContent = t("regionLabel");
  document.querySelector("label[for='file-type']").textContent = t("fileTypeLabel");
  document.querySelector("#file-type option[value='']").textContent = t("fileTypeAny");
  document.querySelector("label[for='local-token']").textContent = t("tokenLabel");
  document.querySelector("#token-help").textContent = t("tokenHelp");
  // The Marginalia provider-credential surface: button labels and the whole
  // dialog resolve through the catalog too, so a non-Spanish locale never shows
  // a half-translated window (the dialog is the one place a secret is typed).
  document.querySelector("#marginalia-key-button").textContent = t("marginaliaKeyButton");
  document.querySelector("#marginalia-clear-button").textContent = t("marginaliaClearButton");
  document.querySelector("#marginalia-dialog-heading").textContent = t("marginaliaDialogHeading");
  document.querySelector("#marginalia-dialog-help").textContent = t("marginaliaDialogHelp");
  document.querySelector("label[for='marginalia-key']").textContent = t("marginaliaApiKeyLabel");
  document.querySelector("label[for='marginalia-ttl']").textContent = t("marginaliaTtlLabel");
  marginaliaKeyInput.placeholder = t("marginaliaKeyPlaceholder");
  document.querySelector("#marginalia-save").textContent = t("marginaliaSave");
  document.querySelector("#marginalia-cancel").textContent = t("marginaliaCancel");
  updateFiltersSummary();
  updateAccessSummary();
  updateThemeToggle();
}

// Applied on load, and again on every explicit language change.
document.addEventListener("DOMContentLoaded", () => {
  uiLanguage.value = LANG;
  applyTranslations();
  setStatusKey("initialStatus", "");
  restoreFromLocation();
});
