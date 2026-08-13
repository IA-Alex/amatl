"use strict";

const PAGE_SIZE = 10;
const MAX_DEEP_DOCUMENTS = 20;
const MAX_FRAGMENTS = 8;
const MAX_FRAGMENT_BYTES = 512;
const MAX_DOCUMENT_BYTES = 8 * 1024 * 1024;
const state = { items: [], mode: "search", page: 0, controller: null };
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
  searchButton.disabled = busy;
  deepButton.disabled = busy;
  searchButton.dataset.active = String(busy && mode === "search");
  deepButton.dataset.active = String(busy && mode === "deep");
  queryInput.setAttribute("aria-busy", String(busy));
  loadingNode.hidden = !busy;
  loadingNode.setAttribute("aria-hidden", String(!busy));
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
    values.push(`Fuente: ${boundedText(result.providers[0], 80)}`);
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
  snippet.textContent = boundedText(result.snippet, 2000) || "Sin descripción disponible.";
  const meta = fragment.querySelector(".result-meta");
  meta.textContent = searchMetadata(result);
  meta.hidden = !meta.textContent;
  resultsNode.append(fragment);
}

function documentStatus(value) {
  if (value === "enriched") return "Enriquecido";
  if (value === "superficial") return "Superficial";
  return "No disponible";
}

function acquisitionMethod(value) {
  if (value === "http") return "HTTP";
  if (value === "rendered") return "Navegador aislado";
  return "No especificada";
}

function signalLabel(value) {
  if (value === "query_match") return "Coincide con la consulta";
  if (value === "citation") return "Incluye enlace";
  if (value === "temporal") return "Referencia temporal";
  if (value === "numeric") return "Dato numérico";
  return null;
}

function appendSafeUrl(node, value) {
  const url = safeHttpUrl(value);
  if (!url) {
    node.textContent = "No disponible";
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
  if (result === "verified") node.textContent = "Rango y hash verificados";
  else if (result === "range_only") node.textContent = "Rango verificado";
  else node.textContent = "No verificable";
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
  hash.textContent = validHash(fragment.fragment_hash) ? fragment.fragment_hash : "No disponible";
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
    values.push(`${documentValue.size.toLocaleString("es-MX")} bytes`);
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
    link.textContent = boundedText(documentValue.title, 300) || "Documento sin URL válida";
    fragment.querySelector(".result-url").textContent = "Origen no disponible";
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
  const provenanceId = typeof provenance.provenance_id === "string" ? provenance.provenance_id : "";
  let sourceBytes = null;
  if (typeof documentValue.content === "string" && documentValue.content.length <= MAX_DOCUMENT_BYTES) {
    const encoded = new TextEncoder().encode(documentValue.content);
    if (encoded.length <= MAX_DOCUMENT_BYTES) sourceBytes = encoded;
  }
  for (const evidenceFragment of fragments) {
    renderEvidenceFragment(evidenceFragment, sourceBytes, list);
  }
  const renderedCount = list.childElementCount;
  fragment.querySelector(".fragment-count").textContent = `${renderedCount} de ${MAX_FRAGMENTS} máx.`;
  if (!renderedCount) {
    empty.hidden = false;
    empty.textContent = documentValue.status === "superficial"
      ? "Documento superficial: no existe texto extraído para citar."
      : "No se obtuvieron fragmentos verificables para este documento.";
  }

  appendSafeUrl(fragment.querySelector(".provenance-original"), provenance.original_url);
  appendSafeUrl(fragment.querySelector(".provenance-canonical"), provenance.canonical_url);
  appendSafeUrl(fragment.querySelector(".provenance-final"), provenance.final_url);
  fragment.querySelector(".provenance-method").textContent = acquisitionMethod(provenance.fetch_method);
  fragment.querySelector(".provenance-extractor").textContent = boundedText(provenance.extractor_used, 160) || "Sin extractor";
  fragment.querySelector(".provenance-retrieved").textContent = boundedText(provenance.retrieved_at, 64) || "No disponible";
  fragment.querySelector(".provenance-source-hash").textContent = validHash(provenance.source_content_hash)
    ? provenance.source_content_hash
    : "No disponible";
  fragment.querySelector(".provenance-content-hash").textContent = validHash(provenance.extracted_content_hash)
    ? provenance.extracted_content_hash
    : "No disponible";
  resultsNode.append(fragment);
}

function render() {
  resultsNode.replaceChildren();
  const start = state.page * PAGE_SIZE;
  const visible = state.items.slice(start, start + PAGE_SIZE);
  if (state.mode === "deep") {
    for (const item of visible) renderDeepDocument(item);
  } else {
    for (const result of visible) renderSearchResult(result);
  }
  const pages = Math.max(1, Math.ceil(state.items.length / PAGE_SIZE));
  paginationNode.hidden = pages <= 1;
  pageLabel.textContent = `Página ${state.page + 1} de ${pages}`;
  previousButton.disabled = state.page === 0;
  nextButton.disabled = state.page + 1 >= pages;
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

async function execute(event) {
  event.preventDefault();
  if (!form.reportValidity()) return;
  const mode = event.submitter?.dataset.mode === "deep" ? "deep" : "search";
  if (state.controller) state.controller.abort();
  const controller = new AbortController();
  state.controller = controller;
  state.items = [];
  state.mode = mode;
  state.page = 0;
  resultHeading.textContent = mode === "deep" ? "Evidencia" : "Resultados";
  resultsNode.replaceChildren();
  paginationNode.hidden = true;
  setBusy(true, mode);
  setStatus(mode === "deep" ? "Analizando y extrayendo evidencia…" : "Buscando…", "loading");
  try {
    const headers = {
      Accept: "application/json",
      "Content-Type": "application/json",
    };
    if (tokenInput.value) headers.Authorization = `Bearer ${tokenInput.value}`;
    const endpoint = mode === "deep" ? "/deep" : "/search";
    const response = await fetch(endpoint, {
      method: "POST",
      headers,
      body: JSON.stringify({ q: queryText() }),
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
        setStatus("No se pudo extraer evidencia; revisa las degradaciones de Deep.", "partial");
      } else if (!fragments || (Array.isArray(payload.degradations) && payload.degradations.length)) {
        setStatus(`${state.items.length} documentos; evidencia parcial o degradada.`, "partial");
      } else {
        setStatus(`${state.items.length} documentos y ${fragments} fragmentos de evidencia.`, "success");
      }
    } else {
      state.items = payload.results;
      render();
      if (!state.items.length) {
        setStatus("No se encontraron resultados.", "partial");
      } else if (payload.status === "partial_success") {
        setStatus(`${state.items.length} resultados; algunas fuentes no respondieron.`, "partial");
      } else {
        setStatus(`${state.items.length} resultados.`, "success");
      }
    }
    resultsNode.focus({ preventScroll: true });
  } catch (error) {
    if (error.name !== "AbortError") {
      if (error.message === "unauthorized") {
        setStatus("El token de acceso falta o no es válido.", "error");
        tokenInput.focus();
      } else if (error.message === "rate_limited") {
        setStatus("Se alcanzó el límite temporal de solicitudes. Espera un minuto.", "error");
      } else if (error.message === "timeout") {
        setStatus("La operación excedió el tiempo permitido.", "error");
      } else {
        setStatus("No fue posible completar la operación. Intenta nuevamente.", "error");
      }
    }
  } finally {
    if (state.controller === controller) {
      state.controller = null;
      setBusy(false, mode);
    }
  }
}

form.addEventListener("submit", execute);
window.addEventListener("pagehide", () => {
  if (state.controller) state.controller.abort();
  tokenInput.value = "";
});
previousButton.addEventListener("click", () => {
  if (state.page > 0) state.page -= 1;
  render();
  resultsNode.focus();
});
nextButton.addEventListener("click", () => {
  if ((state.page + 1) * PAGE_SIZE < state.items.length) state.page += 1;
  render();
  resultsNode.focus();
});
