"use strict";

// ── Theme ───────────────────────────────────────────────────────
// Applied as the very first thing this file does, ahead of i18n and every
// other module below: the CSP (script-src 'self', no inline scripts) rules
// out the usual head-of-document inline snippet that would set this before
// first paint, so the next best thing is doing it before anything else in
// the one script that does run.
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
// Catalogs live in /i18n.js so this file holds behavior only.
const CATALOG = globalThis.AMATL_LOCALES || { defaultLocale: "en", locales: { en: {} } };
const FALLBACK = CATALOG.locales[CATALOG.defaultLocale] || {};
const LANG = (() => {
  const nav = ((typeof navigator !== "undefined" && navigator.language) || "").slice(0, 2);
  return Object.prototype.hasOwnProperty.call(CATALOG.locales, nav) ? nav : CATALOG.defaultLocale;
})();

function t(key) {
  return CATALOG.locales[LANG]?.[key] || FALLBACK[key] || key;
}

const PAGE_SIZE = 10;
const MAX_DEEP_DOCUMENTS = 20;
const MAX_FRAGMENTS = 8;
const MAX_FRAGMENT_BYTES = 512;
const MAX_DOCUMENT_BYTES = 8 * 1024 * 1024;
const MAX_SAVED_PAYLOAD_BYTES = 1024 * 1024;
const LIST_LIMIT = 20;

// Pagination is always server-side: the service returns total_results, page
// and page_size for every search, and this page never re-windows locally.
const state = {
  items: [],
  mode: "search",
  page: 0,
  totalPages: 1,
  totalResults: 0,
  controller: null,
  answer: null,
};
const form = document.querySelector("#search-form");
const queryInput = document.querySelector("#query");
const languageInput = document.querySelector("#language");
const regionInput = document.querySelector("#region");
const fileTypeInput = document.querySelector("#file-type");
const tokenInput = document.querySelector("#local-token");
const searchButton = document.querySelector("#search-button");
const deepButton = document.querySelector("#deep-button");
const answerButton = document.querySelector("#answer-button");
const answerHint = document.querySelector("#answer-hint");
const answerCard = document.querySelector("#answer-card");
const answerTextNode = document.querySelector("#answer-text");
const answerMetaNode = document.querySelector("#answer-meta");
const answerConfig = document.querySelector("#answer-config");
const answerConfigBody = document.querySelector("#answer-config-body");
const answerEnabledToggle = document.querySelector("#answer-enabled-toggle");
const answerToggleStatus = document.querySelector("#answer-toggle-status");
const resultHeading = document.querySelector("#result-heading");
const statusNode = document.querySelector("#status");
const loadingNode = document.querySelector("#loading");
const resultsNode = document.querySelector("#results");
const paginationNode = document.querySelector("#pagination");
const previousButton = document.querySelector("#previous");
const nextButton = document.querySelector("#next");
const pageLabel = document.querySelector("#page-label");
const resultTemplate = document.querySelector("#result-template");
const deepTemplate = document.querySelector("#deep-template");
const fragmentTemplate = document.querySelector("#fragment-template");
const cancelButton = document.querySelector("#cancel-button");
const themeToggle = document.querySelector("#theme-toggle");

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
  } catch {
    // Private browsing or a full storage quota: the toggle still works for
    // this page load, it just won't be remembered on the next visit.
  }
  updateThemeToggle();
});

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

function authHeaders(extra) {
  const headers = Object.assign({ Accept: "application/json" }, extra || {});
  if (tokenInput.value) headers.Authorization = `Bearer ${tokenInput.value}`;
  return headers;
}

function setBusy(busy, mode) {
  searchButton.disabled = busy;
  deepButton.disabled = busy;
  searchButton.dataset.active = String(busy && mode === "search");
  deepButton.dataset.active = String(busy && mode === "deep");
  queryInput.setAttribute("aria-busy", String(busy));
  loadingNode.hidden = !busy;
  loadingNode.setAttribute("aria-hidden", String(!busy));
  cancelButton.hidden = !busy;
  cancelButton.setAttribute("aria-hidden", String(!busy));
}

function setStatus(message, status) {
  statusNode.textContent = message;
  statusNode.dataset.state = status;
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

function searchMetadata(result) {
  const values = [];
  if (typeof result.published_at === "string") values.push(result.published_at.slice(0, 64));
  if (Array.isArray(result.providers) && result.providers.length === 1) {
    values.push(`${t("source")} ${boundedText(result.providers[0], 80)}`);
  }
  return values.join(" · ");
}

function renderSearchResult(result, citationIndex) {
  const url = safeHttpUrl(result.canonical_url);
  if (!url || result.status !== "visible") return;
  const fragment = resultTemplate.content.cloneNode(true);
  const link = fragment.querySelector(".result-title");
  link.href = url.href;
  link.textContent = boundedText(result.title, 300) || boundedText(result.domain, 255) || url.hostname;
  fragment.querySelector(".result-url").textContent = boundedText(result.domain, 255) || url.hostname;
  const snippet = fragment.querySelector(".result-snippet");
  snippet.textContent = boundedText(result.snippet, 2000) || t("noDescription");
  const meta = fragment.querySelector(".result-meta");
  meta.textContent = searchMetadata(result);
  meta.hidden = !meta.textContent;
  // Only in answer mode, and only the true 1-based position in the source
  // array AMATL sent the model — not a post-filter counter — so it matches
  // the [n] markers in the answer text exactly, even when an earlier item
  // was skipped above.
  if (state.mode === "answer" && Number.isInteger(citationIndex)) {
    const badge = fragment.querySelector(".result-index");
    badge.textContent = `[${citationIndex}]`;
    badge.hidden = false;
  }
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
  const observed = Array.isArray(fragment.signals) ? fragment.signals.slice(0, 8) : [];
  for (const signal of observed) {
    const label = signalLabel(signal);
    if (!label) continue;
    const chip = document.createElement("span");
    chip.textContent = label;
    signals.append(chip);
  }
  if (!signals.childElementCount) signals.hidden = true;
  const hash = item.querySelector(".fragment-hash code");
  hash.textContent = validHash(fragment.fragment_hash) ? fragment.fragment_hash : t("unavailable");
  const verification = item.querySelector(".fragment-verification");
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
  saveButton.addEventListener("click", () => saveDocument(documentValue, provenance, saveButton));
  resultsNode.append(fragment);
}

function renderAnswerCard(answer) {
  if (!answer || typeof answer.text !== "string") {
    answerCard.hidden = true;
    return;
  }
  answerTextNode.textContent = answer.text;
  const citations = Array.isArray(answer.citations) ? answer.citations.length : 0;
  answerMetaNode.textContent = `${t("answerSourcesNote")} ${citations} — ${boundedText(answer.model, 120)}`;
  answerCard.hidden = false;
}

function render() {
  resultsNode.replaceChildren();
  if (state.mode === "deep") {
    for (const item of state.items) renderDeepDocument(item);
    paginationNode.hidden = true;
    return;
  }
  if (state.mode === "answer") {
    renderAnswerCard(state.answer);
    state.items.forEach((result, i) => renderSearchResult(result, i + 1));
    paginationNode.hidden = true;
    return;
  }
  for (const result of state.items) renderSearchResult(result);
  paginationNode.hidden = state.totalPages <= 1;
  pageLabel.textContent = `${t("pageOf")} ${state.page + 1} ${t("of")} ${state.totalPages}`;
  previousButton.disabled = state.page === 0;
  nextButton.disabled = state.page + 1 >= state.totalPages;
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
  if (mode === "answer") {
    return (
      payload.answer
      && typeof payload.answer.text === "string"
      && Array.isArray(payload.answer.citations)
      && payload.search
      && Array.isArray(payload.search.results)
    );
  }
  return Array.isArray(payload.results);
}

function statusFromHttp(response) {
  if (response.status === 401) return "unauthorized";
  if (response.status === 429) return "rate_limited";
  if (response.status === 504) return "timeout";
  return "request_failed";
}

function reportFailure(error) {
  if (error.name === "AbortError") return;
  if (error.message === "unauthorized") {
    setStatus(t("unauthorized"), "error");
    tokenInput.focus();
  } else if (error.message === "rate_limited") {
    setStatus(t("rateLimited"), "error");
  } else if (error.message === "timeout") {
    setStatus(t("timeout"), "error");
  } else {
    setStatus(t("genericError"), "error");
  }
}

async function run(mode, page) {
  if (state.controller) state.controller.abort();
  const controller = new AbortController();
  state.controller = controller;
  state.items = [];
  state.mode = mode;
  state.page = mode === "deep" ? 0 : page;
  resultHeading.textContent = mode === "deep"
    ? t("evidenceHeading")
    : mode === "answer"
      ? t("answerHeading")
      : t("resultsHeading");
  resultsNode.replaceChildren();
  answerCard.hidden = true;
  paginationNode.hidden = true;
  setBusy(true, mode);
  setStatus(
    mode === "deep" ? t("analyzing") : mode === "answer" ? t("answerGenerating") : t("searching"),
    "loading",
  );
  try {
    const headers = authHeaders({ "Content-Type": "application/json" });
    const endpoint = mode === "deep" ? "/deep" : mode === "answer" ? "/answer" : "/search";
    const body = { q: queryText() };
    if (mode === "search") {
      body.page = state.page;
      body.page_size = PAGE_SIZE;
    }
    const response = await fetch(endpoint, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      credentials: "same-origin",
      signal: controller.signal,
    });
    if (!response.ok) throw new Error(statusFromHttp(response));
    const payload = await response.json();
    if (state.controller !== controller) return;
    if (!validatePayload(payload, mode)) throw new Error("invalid_contract");
    if (mode === "deep") {
      state.items = deepItems(payload);
      render();
      const fragments = state.items.reduce(
        (total, item) => total + evidenceFragments(item.evidence).length,
        0,
      );
      if (!state.items.length) {
        setStatus(t("noEvidence"), "partial");
      } else if (!fragments || (Array.isArray(payload.degradations) && payload.degradations.length)) {
        setStatus(`${state.items.length} ${t("partialEvidence")}`, "partial");
      } else {
        setStatus(`${state.items.length} ${t("evidenceCount")} ${fragments} ${t("evidenceFragments")}`, "success");
      }
    } else if (mode === "answer") {
      state.items = payload.search.results;
      state.answer = payload.answer;
      render();
      setStatus(`${t("answerDone")} ${payload.answer.citations.length} ${t("answerCitations")}`, "success");
    } else {
      state.items = payload.results;
      const pageSize = typeof payload.page_size === "number" && payload.page_size > 0
        ? payload.page_size
        : PAGE_SIZE;
      state.totalResults = typeof payload.total_results === "number"
        ? payload.total_results
        : state.items.length;
      state.page = typeof payload.page === "number" ? payload.page : state.page;
      state.totalPages = Math.max(1, Math.ceil(state.totalResults / pageSize));
      render();
      if (!state.items.length) {
        setStatus(t("noResults"), "partial");
      } else if (payload.status === "partial_success") {
        setStatus(`${state.items.length} ${t("partialResults")}`, "partial");
      } else {
        setStatus(`${state.items.length} ${t("of")} ${state.totalResults} ${t("resultsCount")}`, "success");
      }
    }
    resultsNode.focus({ preventScroll: true });
    refreshPanels();
  } catch (error) {
    reportFailure(error);
  } finally {
    if (state.controller === controller) {
      state.controller = null;
      setBusy(false, mode);
    }
  }
}

function execute(event) {
  event.preventDefault();
  if (requestedModeIsUnavailable(event.submitter)) {
    setStatus(t("answerHintMissingCredential"), "error");
    return;
  }
  if (!form.reportValidity()) return;
  const requested = event.submitter?.dataset.mode;
  const mode = requested === "deep" || requested === "answer" ? requested : "search";
  run(mode, 0);
}

function requestedModeIsUnavailable(submitter) {
  return submitter === answerButton && answerButton.classList.contains("is-unavailable");
}

form.addEventListener("submit", execute);
cancelButton.addEventListener("click", () => {
  if (state.controller) {
    state.controller.abort();
    setStatus(t("cancelled"), "partial");
  }
});
window.addEventListener("pagehide", () => {
  if (state.controller) state.controller.abort();
  tokenInput.value = "";
});
previousButton.addEventListener("click", () => {
  if (state.page > 0) run("search", state.page - 1);
});
nextButton.addEventListener("click", () => {
  if (state.page + 1 < state.totalPages) run("search", state.page + 1);
});

// ── Source availability ─────────────────────────────────────────
const providerStatus = document.querySelector(".provider-status");
const providerList = document.querySelector("#provider-list");
const providerStatusText = document.querySelector("#provider-status-text");

// Maps the `code` a provider's availability carries (amatl-core
// `ProviderAvailability::Unavailable`) to a catalog key with a plain-language
// reason. Unrecognized codes still get a visible, if generic, explanation
// instead of silence — the raw code stays in the title attribute for support.
const PROVIDER_REASON_KEYS = {
  provider_not_approved: "providerReasonNotApproved",
  provider_disabled: "providerReasonDisabled",
  provider_credential_missing: "providerReasonCredentialMissing",
  credential_missing: "providerReasonCredentialMissing",
  provider_circuit_open: "providerReasonCircuitOpen",
  egress_denied: "providerReasonEgressDenied",
};

function providerReason(code) {
  const key = code ? PROVIDER_REASON_KEYS[code] : undefined;
  return t(key ?? "providerReasonUnknown");
}

// Admin-only: POST an `{ enabled }` flip to the server, refresh the relevant
// surface, and on failure roll the checkbox back and surface the error.
// Shared by `toggleAnswer` and `toggleProvider` so the disabled-while-in-flight
// discipline, the rollback and the i18n of failures live in one place instead
// of drifting apart.
async function toggleEnabled({ endpoint, desired, checkbox, statusNode, refresh, failKey }) {
  checkbox.disabled = true;
  if (statusNode) statusNode.hidden = true;
  try {
    const response = await fetch(endpoint, {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({ enabled: desired }),
      credentials: "same-origin",
    });
    if (!response.ok) throw new Error(statusFromHttp(response));
    await refresh();
  } catch (error) {
    checkbox.checked = !desired;
    checkbox.disabled = false;
    if (statusNode) statusNode.hidden = false;
    statusNode.textContent =
      error.message === "unauthorized" ? t("unauthorized") : t(failKey);
  }
}

// Admin-only: flips whether `name` is in `providers.enabled`, persisted to
// the config file and applied without a restart. Never touches the
// provider's governance ficha — approval, credential and terms stay exactly
// as they were; see `Config::set_provider_enabled`.
async function toggleProvider(name, desired, checkbox, reasonNode) {
  await toggleEnabled({
    endpoint: `/providers/${encodeURIComponent(name)}/enabled`,
    desired,
    checkbox,
    statusNode: reasonNode,
    refresh: fetchProviderStatus,
    failKey: "providerToggleFailed",
  });
}

async function fetchProviderStatus() {
  try {
    const response = await fetch("/providers", {
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (!response.ok) return;
    const payload = await response.json();
    if (!payload || !Array.isArray(payload.providers)) return;
    providerList.replaceChildren();
    let unavailable = 0;
    for (const provider of payload.providers) {
      const item = document.createElement("li");
      item.className = "provider-item";
      const row = document.createElement("div");
      row.className = "provider-item-row";
      const name = document.createElement("span");
      name.className = "provider-name";
      name.textContent = provider.name;
      const badge = document.createElement("span");
      badge.className = `provider-badge provider-badge--${provider.status}`;
      badge.textContent = provider.status === "available" ? t("available") : t("notAvailable");
      if (provider.code) badge.title = provider.code;
      const toggleLabel = document.createElement("label");
      toggleLabel.className = "provider-toggle";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = Boolean(provider.enabled);
      checkbox.setAttribute("aria-label", `${t("providerToggleLabel")} ${provider.name}`);
      const toggleText = document.createElement("span");
      toggleText.textContent = t("providerToggleLabel");
      toggleLabel.append(checkbox, toggleText);
      row.append(name, badge, toggleLabel);
      item.append(row);
      const reason = document.createElement("span");
      reason.className = "provider-reason";
      if (provider.status !== "available") {
        reason.textContent = providerReason(provider.code);
        unavailable += 1;
      } else {
        reason.hidden = true;
      }
      item.append(reason);
      checkbox.addEventListener("change", () =>
        toggleProvider(provider.name, checkbox.checked, checkbox, reason),
      );
      providerList.append(item);
    }
    providerStatus.hidden = false;
    if (unavailable === 0) {
      providerStatusText.textContent = t("allAvailable");
      providerStatusText.dataset.state = "success";
    } else if (unavailable < payload.providers.length) {
      providerStatusText.textContent = `${unavailable} ${t("someUnavailable")}`;
      providerStatusText.dataset.state = "partial";
    } else {
      providerStatusText.textContent = t("noneAvailable");
      providerStatusText.dataset.state = "error";
    }
  } catch (_) {
    // Silently ignore; source availability is best-effort.
  }
}

// ── Service state ───────────────────────────────────────────────
const servicePanel = document.querySelector(".service-state");
const serviceSummary = document.querySelector("#service-summary");
const serviceIndicators = document.querySelector("#service-indicators");

function indicator(label, value, indicatorState) {
  const item = document.createElement("li");
  item.className = "indicator";
  const name = document.createElement("span");
  name.className = "indicator-label";
  name.textContent = label;
  const reading = document.createElement("span");
  reading.className = "indicator-value";
  reading.dataset.state = indicatorState;
  reading.textContent = value;
  item.append(name, reading);
  return item;
}

function configRow(label, value) {
  const wrapper = document.createElement("div");
  const dt = document.createElement("dt");
  dt.textContent = label;
  const dd = document.createElement("dd");
  dd.textContent = value;
  wrapper.append(dt, dd);
  return wrapper;
}

function percentage(value) {
  return `${Math.round((typeof value === "number" ? value : 0) * 100)}%`;
}

async function fetchServiceState() {
  try {
    const response = await fetch("/status", {
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (!response.ok) {
      servicePanel.hidden = true;
      return null;
    }
    const payload = await response.json();
    if (!payload || payload.schema_version !== "1") {
      servicePanel.hidden = true;
      return null;
    }
    const sources = Array.isArray(payload.sources) ? payload.sources : [];
    const availableSources = sources.filter((source) => source.status === "available").length;
    const storage = payload.storage && typeof payload.storage === "object" ? payload.storage : {};
    const cache = payload.cache && typeof payload.cache === "object" ? payload.cache : {};
    serviceIndicators.replaceChildren();
    serviceIndicators.append(
      indicator(
        t("sourcesLabel"),
        `${availableSources}/${sources.length}`,
        availableSources === sources.length && sources.length ? "success" : "partial",
      ),
    );
    let storageLabel = t("storageOff");
    let storageState = "partial";
    if (storage.enabled && storage.available) {
      storageLabel = t("storageOn");
      storageState = "success";
    } else if (storage.enabled) {
      storageLabel = t("storageBroken");
      storageState = "error";
    }
    serviceIndicators.append(indicator(t("storageLabel"), storageLabel, storageState));
    serviceIndicators.append(
      indicator(t("cacheLabel"), percentage(cache.provider_search_hit_rate), "success"),
    );
    const answerStatus = payload.answer && typeof payload.answer === "object" ? payload.answer : {};
    let answerLabel = t("storageOff");
    let answerState = "partial";
    if (answerStatus.available) {
      answerLabel = t("storageOn");
      answerState = "success";
    } else if (answerStatus.enabled) {
      answerLabel = t("storageBroken");
      answerState = "error";
    }
    serviceIndicators.append(indicator(t("answerButton"), answerLabel, answerState));
    // Always visible, never truly `disabled`: a disabled button gives no
    // hover/touch feedback and no reason why. This one stays clickable —
    // execute() intercepts the click and explains instead of submitting.
    answerButton.classList.toggle("is-unavailable", !answerStatus.available);
    answerButton.setAttribute("aria-disabled", String(!answerStatus.available));
    // Lives right by the search buttons, not just in the status panel below
    // the fold: that panel is easy to miss, and this is the one place an
    // operator will actually look when the button they expect isn't there.
    answerHint.hidden = !(answerStatus.enabled && !answerStatus.available);
    answerHint.textContent = t("answerHintMissingCredential");
    // `configured` (endpoint+model on disk) is independent of `enabled`, on
    // purpose: the toggle below needs a real setting to switch even while
    // it's off, or turning the feature back on would never be reachable.
    if (answerStatus.configured) {
      answerConfigBody.replaceChildren();
      answerConfigBody.append(
        configRow(t("answerConfigStateLabel"), answerLabel),
        configRow(t("answerConfigModelLabel"), boundedText(answerStatus.model, 160) || t("unavailable")),
        configRow(t("answerConfigEndpointLabel"), boundedText(answerStatus.endpoint, 160) || t("unavailable")),
      );
      answerConfig.hidden = false;
      answerEnabledToggle.checked = Boolean(answerStatus.enabled);
      answerEnabledToggle.disabled = false;
    } else {
      answerConfig.hidden = true;
    }
    const degraded = payload.status !== "ok";
    serviceSummary.textContent = degraded ? t("serviceDegraded") : t("serviceOk");
    serviceSummary.dataset.state = degraded ? "partial" : "success";
    servicePanel.hidden = false;
    return payload;
  } catch (_) {
    servicePanel.hidden = true;
    return null;
  }
}

// Admin-only: flips just `answer.enabled` on the server, persisted to its
// config file and applied without a restart. Never touches the credential,
// provider, or model — see `docs/resumen-con-ia.md`.
async function toggleAnswer() {
  await toggleEnabled({
    endpoint: "/answer/enabled",
    desired: answerEnabledToggle.checked,
    checkbox: answerEnabledToggle,
    statusNode: answerToggleStatus,
    refresh: fetchServiceState,
    failKey: "answerToggleFailed",
  });
}

answerEnabledToggle.addEventListener("change", toggleAnswer);

// ── History ─────────────────────────────────────────────────────
const historyPanel = document.querySelector(".history-panel");
const historyList = document.querySelector("#history-list");
const historyEmpty = document.querySelector("#history-empty");
const historyPurgeButton = document.querySelector("#history-purge");
const historyTemplate = document.querySelector("#history-template");

function historyLabel(entry) {
  const parts = [];
  if (Number.isSafeInteger(entry.total_results)) parts.push(`${entry.total_results} ${t("resultsCount")}`);
  if (Number.isSafeInteger(entry.created_at) && entry.created_at > 0) {
    parts.push(new Date(entry.created_at * 1000).toLocaleString(LANG));
  }
  return parts.join(" · ");
}

async function fetchHistory() {
  try {
    const response = await fetch(`/history?limit=${LIST_LIMIT}`, {
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (!response.ok) {
      historyPanel.hidden = true;
      return;
    }
    const payload = await response.json();
    if (!payload || payload.schema_version !== "1" || !Array.isArray(payload.entries)) {
      historyPanel.hidden = true;
      return;
    }
    historyList.replaceChildren();
    for (const entry of payload.entries.slice(0, LIST_LIMIT)) {
      if (!entry || !Number.isSafeInteger(entry.id)) continue;
      const item = historyTemplate.content.cloneNode(true);
      item.querySelector(".history-query").textContent = boundedText(entry.raw_query, 300);
      item.querySelector(".history-meta").textContent = historyLabel(entry);
      const reuse = item.querySelector(".history-reuse");
      reuse.textContent = t("historyReuse");
      reuse.addEventListener("click", () => {
        queryInput.value = boundedText(entry.raw_query, 2048);
        queryInput.focus();
      });
      const remove = item.querySelector(".history-delete");
      remove.textContent = t("historyDelete");
      remove.addEventListener("click", () => deleteHistoryEntry(entry.id));
      historyList.append(item);
    }
    historyEmpty.hidden = historyList.childElementCount > 0;
    historyEmpty.textContent = t("historyEmpty");
    historyPanel.hidden = false;
  } catch (_) {
    historyPanel.hidden = true;
  }
}

async function deleteHistoryEntry(id) {
  try {
    const response = await fetch(`/history/${id}`, {
      method: "DELETE",
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (response.ok) setStatus(t("historyDeleted"), "success");
    await fetchHistory();
  } catch (_) {
    setStatus(t("genericError"), "error");
  }
}

async function purgeHistory() {
  try {
    const response = await fetch("/history", {
      method: "DELETE",
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (response.ok) setStatus(t("historyPurged"), "success");
    await fetchHistory();
  } catch (_) {
    setStatus(t("genericError"), "error");
  }
}

// ── Saved documents ─────────────────────────────────────────────
const savedPanel = document.querySelector(".saved-panel");
const savedList = document.querySelector("#saved-list");
const savedEmpty = document.querySelector("#saved-empty");
const savedTemplate = document.querySelector("#saved-template");

async function fetchSavedDocuments() {
  try {
    const response = await fetch(`/saved?limit=${LIST_LIMIT}`, {
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (!response.ok) {
      savedPanel.hidden = true;
      return;
    }
    const payload = await response.json();
    if (!payload || payload.schema_version !== "1" || !Array.isArray(payload.documents)) {
      savedPanel.hidden = true;
      return;
    }
    savedList.replaceChildren();
    for (const saved of payload.documents.slice(0, LIST_LIMIT)) {
      if (!saved || !Number.isSafeInteger(saved.id)) continue;
      const url = safeHttpUrl(saved.canonical_url);
      const item = savedTemplate.content.cloneNode(true);
      const link = item.querySelector(".saved-title");
      link.textContent = boundedText(saved.title, 300) || (url ? url.hostname : t("noUrl"));
      if (url) link.href = url.href;
      else link.removeAttribute("href");
      item.querySelector(".saved-meta").textContent = boundedText(saved.snippet, 200);
      const remove = item.querySelector(".saved-delete");
      remove.textContent = t("savedDelete");
      remove.addEventListener("click", () => deleteSavedDocument(saved.id));
      savedList.append(item);
    }
    savedEmpty.hidden = savedList.childElementCount > 0;
    savedEmpty.textContent = t("savedEmpty");
    savedPanel.hidden = false;
  } catch (_) {
    savedPanel.hidden = true;
  }
}

async function deleteSavedDocument(id) {
  try {
    const response = await fetch(`/saved/${id}`, {
      method: "DELETE",
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (response.ok) setStatus(t("savedDeleted"), "success");
    await fetchSavedDocuments();
  } catch (_) {
    setStatus(t("genericError"), "error");
  }
}

/// Persist one Deep document; the payload is the document as served, bounded
/// so the UI never posts more than the service accepts.
async function saveDocument(documentValue, provenance, button) {
  const url = safeHttpUrl(documentValue.canonical_url);
  if (!url || !validHash(documentValue.content_hash)) {
    setStatus(t("savedFailed"), "error");
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
    setStatus(t("savedFailed"), "error");
    return;
  }
  button.disabled = true;
  try {
    const response = await fetch("/saved", {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      credentials: "same-origin",
      body: JSON.stringify({
        canonical_url: url.href,
        title: boundedText(documentValue.title, 300),
        snippet: boundedText(documentValue.excerpt, 2000),
        content_hash: documentValue.content_hash,
        extractor_version: boundedText(provenance.extractor_used, 128) || "unknown",
        payload,
        source_query: boundedText(queryInput.value, 2048),
        tags: [],
      }),
    });
    if (!response.ok) throw new Error(statusFromHttp(response));
    setStatus(t("savedDone"), "success");
    await fetchSavedDocuments();
  } catch (error) {
    reportFailure(error);
  } finally {
    button.disabled = false;
  }
}

// ── Server clients (admin) ──────────────────────────────────────
// Only reachable with an Admin-scoped token; `fetchServerClients` hides the
// whole panel on anything but a successful `200`, the same fail-closed
// convention `fetchServiceState`/`fetchSavedDocuments` already use — a
// non-admin credential simply never sees this section, rather than seeing
// it and hitting a wall of 403s.
const clientsPanel = document.querySelector(".clients-panel");
const clientList = document.querySelector("#client-list");
const clientListEmpty = document.querySelector("#client-list-empty");
const clientCreateDetails = document.querySelector("#client-create");
const clientCreateForm = document.querySelector("#client-create-form");
const clientIdInput = document.querySelector("#client-id");
const clientExpiresInput = document.querySelector("#client-expires");
const clientCreateError = document.querySelector("#client-create-error");
const clientTokenDialog = document.querySelector("#client-token-dialog");
const clientTokenId = document.querySelector("#client-token-id");
const clientTokenValue = document.querySelector("#client-token-value");
const clientTokenClose = document.querySelector("#client-token-close");

const SCOPE_LABEL_KEYS = {
  search: "scopeSearch",
  deep: "scopeDeep",
  read: "scopeRead",
  write: "scopeWrite",
  admin: "scopeAdmin",
  mcp: "scopeMcp",
};

function scopeLabel(scope) {
  const key = SCOPE_LABEL_KEYS[scope];
  return key ? t(key) : scope;
}

function renderClientRow(client) {
  const item = document.createElement("li");
  item.className = "client-item";
  const row = document.createElement("div");
  row.className = "client-item-row";
  const id = document.createElement("span");
  id.className = "client-id";
  id.textContent = client.id;
  const scopes = document.createElement("span");
  scopes.className = "client-scopes-value";
  const scopeList = Array.isArray(client.scopes) ? client.scopes : [];
  scopes.textContent = scopeList.length
    ? scopeList.map(scopeLabel).join(", ")
    : t("clientScopesEmpty");
  row.append(id, scopes);
  item.append(row);

  const meta = document.createElement("p");
  meta.className = "client-meta";
  meta.textContent =
    typeof client.expires_at === "string" && client.expires_at
      ? `${t("clientExpiresLabelRow")}: ${boundedText(client.expires_at, 32)}`
      : t("clientNeverExpires");
  item.append(meta);

  const actions = document.createElement("div");
  actions.className = "client-actions";
  const rotate = document.createElement("button");
  rotate.type = "button";
  rotate.className = "ghost-action";
  rotate.textContent = t("clientRotate");
  rotate.addEventListener("click", () => rotateServerClient(client.id, rotate));
  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "ghost-action";
  remove.textContent = t("clientDelete");
  remove.addEventListener("click", () => deleteServerClient(client.id, remove));
  actions.append(rotate, remove);
  item.append(actions);
  return item;
}

async function fetchServerClients() {
  try {
    const response = await fetch("/server/clients", {
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (!response.ok) {
      clientsPanel.hidden = true;
      return;
    }
    const payload = await response.json();
    if (!payload || payload.schema_version !== "1" || !Array.isArray(payload.clients)) {
      clientsPanel.hidden = true;
      return;
    }
    clientList.replaceChildren();
    for (const client of payload.clients) {
      if (!client || typeof client.id !== "string") continue;
      clientList.append(renderClientRow(client));
    }
    clientListEmpty.hidden = clientList.childElementCount > 0;
    clientListEmpty.textContent = t("clientsEmpty");
    clientsPanel.hidden = false;
  } catch (_) {
    clientsPanel.hidden = true;
  }
}

// The one place the plaintext token is ever visible: it never leaves this
// dialog, never touches `localStorage`, and the field is cleared the moment
// the operator closes it (see `client-token-close`'s listener below).
function openClientTokenDialog(id, token) {
  clientTokenId.textContent = id;
  clientTokenValue.value = token;
  clientTokenDialog.showModal();
  clientTokenValue.focus();
  clientTokenValue.select();
}

clientTokenClose.addEventListener("click", () => {
  clientTokenValue.value = "";
  clientTokenDialog.close();
});

async function createServerClient(event) {
  event.preventDefault();
  const id = clientIdInput.value.trim();
  const scopes = Array.from(
    clientCreateForm.querySelectorAll('input[name="scope"]:checked'),
  ).map((input) => input.value);
  clientCreateError.hidden = true;
  if (!id || scopes.length === 0) {
    clientCreateError.textContent = t("clientCreateMissingFields");
    clientCreateError.hidden = false;
    return;
  }
  const submitButton = clientCreateForm.querySelector("#client-create-submit");
  submitButton.disabled = true;
  try {
    const body = { id, scopes, tools: [] };
    if (clientExpiresInput.value) body.expires_at = clientExpiresInput.value;
    const response = await fetch("/server/clients", {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      credentials: "same-origin",
      body: JSON.stringify(body),
    });
    if (response.status === 409) throw new Error("duplicate");
    if (!response.ok) throw new Error(statusFromHttp(response));
    const payload = await response.json();
    clientCreateForm.reset();
    clientCreateDetails.open = false;
    await fetchServerClients();
    if (payload && typeof payload.token === "string" && typeof payload.id === "string") {
      openClientTokenDialog(payload.id, payload.token);
    }
  } catch (error) {
    clientCreateError.textContent =
      error.message === "duplicate"
        ? t("clientCreateDuplicate")
        : error.message === "unauthorized"
          ? t("unauthorized")
          : t("clientCreateFailed");
    clientCreateError.hidden = false;
  } finally {
    submitButton.disabled = false;
  }
}

async function deleteServerClient(id, button) {
  if (!globalThis.confirm(t("clientDeleteConfirm"))) return;
  button.disabled = true;
  try {
    const response = await fetch(`/server/clients/${encodeURIComponent(id)}`, {
      method: "DELETE",
      headers: authHeaders(),
      credentials: "same-origin",
    });
    setStatus(response.ok ? t("clientDeleted") : t("clientDeleteFailed"), response.ok ? "success" : "error");
    await fetchServerClients();
  } catch (_) {
    setStatus(t("clientDeleteFailed"), "error");
    button.disabled = false;
  }
}

async function rotateServerClient(id, button) {
  if (!globalThis.confirm(t("clientRotateConfirm"))) return;
  button.disabled = true;
  try {
    const response = await fetch(`/server/clients/${encodeURIComponent(id)}/rotate`, {
      method: "POST",
      headers: authHeaders(),
      credentials: "same-origin",
    });
    if (!response.ok) throw new Error(statusFromHttp(response));
    const payload = await response.json();
    await fetchServerClients();
    if (payload && typeof payload.token === "string" && typeof payload.id === "string") {
      openClientTokenDialog(payload.id, payload.token);
    }
  } catch (_) {
    setStatus(t("clientRotateFailed"), "error");
    button.disabled = false;
  }
}

clientCreateForm.addEventListener("submit", createServerClient);

function refreshPanels() {
  fetchProviderStatus();
  fetchServiceState();
  fetchHistory();
  fetchSavedDocuments();
  fetchServerClients();
}

historyPurgeButton.addEventListener("click", purgeHistory);
tokenInput.addEventListener("change", refreshPanels);
refreshPanels();

// Apply i18n to static HTML content on load.
document.addEventListener("DOMContentLoaded", () => {
  setStatus(t("initialStatus"), "");
  document.querySelector("#search-heading").textContent = t("searchHeading");
  document.querySelector("#search-button").textContent = t("searchButton");
  document.querySelector("#deep-button").textContent = t("deepButton");
  document.querySelector("#answer-button").textContent = t("answerButton");
  document.querySelector("#answer-config-summary").textContent = t("answerConfigLabel");
  document.querySelector("#answer-toggle-label").textContent = t("answerToggleLabel");
  document.querySelector("#previous").textContent = t("previous");
  document.querySelector("#next").textContent = t("next");
  document.querySelector("#cancel-button").textContent = t("cancel");
  document.querySelector(".skip-link").textContent = t("skipToResults");
  document.querySelector(".brand").setAttribute("aria-label", t("brandLabel"));
  document.querySelector("#result-heading").textContent = t("resultsHeading");
  document.querySelector("#provider-heading").textContent = t("providerHeading");
  document.querySelector("#service-heading").textContent = t("serviceHeading");
  document.querySelector("#history-heading").textContent = t("historyHeading");
  document.querySelector("#history-purge").textContent = t("historyPurge");
  document.querySelector("#saved-heading").textContent = t("savedHeading");
  queryInput.placeholder = t("searchPlaceholder");
  document.querySelector("#filters-summary").textContent = t("filtersLabel");
  document.querySelector("label[for='language']").textContent = t("languageLabel");
  document.querySelector("label[for='region']").textContent = t("regionLabel");
  document.querySelector("label[for='file-type']").textContent = t("fileTypeLabel");
  document.querySelector("#file-type option[value='']").textContent = t("fileTypeAny");
  document.querySelector("label[for='local-token']").textContent = t("tokenLabel");
  document.querySelector("#token-help").textContent = t("tokenHelp");
  document.querySelector("#clients-heading").textContent = t("clientsHeading");
  document.querySelector("#client-create-summary").textContent = t("clientCreateLabel");
  document.querySelector("label[for='client-id']").textContent = t("clientIdLabel");
  document.querySelector("#client-id-help").textContent = t("clientIdHelp");
  document.querySelector("#client-scopes-legend").textContent = t("clientScopesLabel");
  document.querySelector("label[for='client-expires']").textContent = t("clientExpiresLabel");
  document.querySelector("#client-create-submit").textContent = t("clientCreateSubmit");
  for (const option of document.querySelectorAll("#client-scopes .scope-option")) {
    const input = option.querySelector("input");
    option.querySelector("span").textContent = scopeLabel(input.value);
  }
  document.querySelector("#client-token-heading").textContent = t("clientTokenHeading");
  document.querySelector("#client-token-warning").textContent = t("clientTokenWarning");
  document.querySelector("#client-token-close").textContent = t("clientTokenClose");
  updateThemeToggle();
});
