"use strict";

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
};
const form = document.querySelector("#search-form");
const queryInput = document.querySelector("#query");
const languageInput = document.querySelector("#language");
const regionInput = document.querySelector("#region");
const fileTypeInput = document.querySelector("#file-type");
const tokenInput = document.querySelector("#local-token");
const searchButton = document.querySelector("#search-button");
const deepButton = document.querySelector("#deep-button");
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

function renderSearchResult(result) {
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

function render() {
  resultsNode.replaceChildren();
  if (state.mode === "deep") {
    for (const item of state.items) renderDeepDocument(item);
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
  resultHeading.textContent = mode === "deep" ? t("evidenceHeading") : t("resultsHeading");
  resultsNode.replaceChildren();
  paginationNode.hidden = true;
  setBusy(true, mode);
  setStatus(mode === "deep" ? t("analyzing") : t("searching"), "loading");
  try {
    const headers = authHeaders({ "Content-Type": "application/json" });
    const endpoint = mode === "deep" ? "/deep" : "/search";
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
  if (!form.reportValidity()) return;
  const mode = event.submitter?.dataset.mode === "deep" ? "deep" : "search";
  run(mode, 0);
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
      const name = document.createElement("span");
      name.className = "provider-name";
      name.textContent = provider.name;
      const badge = document.createElement("span");
      badge.className = `provider-badge provider-badge--${provider.status}`;
      badge.textContent = provider.status === "available" ? t("available") : t("notAvailable");
      if (provider.code) badge.title = provider.code;
      item.append(name, badge);
      providerList.append(item);
      if (provider.status !== "available") unavailable += 1;
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

function refreshPanels() {
  fetchProviderStatus();
  fetchServiceState();
  fetchHistory();
  fetchSavedDocuments();
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
  document.querySelector("#previous").textContent = t("previous");
  document.querySelector("#next").textContent = t("next");
  document.querySelector("#cancel-button").textContent = t("cancel");
  document.querySelector(".skip-link").textContent = t("skipToResults");
  document.querySelector(".brand").setAttribute("aria-label", t("brandLabel"));
  document.querySelector(".product-label").textContent = t("productLabel");
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
});
