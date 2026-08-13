"use strict";

const PAGE_SIZE = 10;
const state = { results: [], page: 0, controller: null };
const form = document.querySelector("#search-form");
const queryInput = document.querySelector("#query");
const languageInput = document.querySelector("#language");
const regionInput = document.querySelector("#region");
const fileTypeInput = document.querySelector("#file-type");
const tokenInput = document.querySelector("#local-token");
const submitButton = document.querySelector("#search-button");
const statusNode = document.querySelector("#status");
const loadingNode = document.querySelector("#loading");
const resultsNode = document.querySelector("#results");
const paginationNode = document.querySelector("#pagination");
const previousButton = document.querySelector("#previous");
const nextButton = document.querySelector("#next");
const pageLabel = document.querySelector("#page-label");
const template = document.querySelector("#result-template");

function safeHttpUrl(value) {
  try {
    const parsed = new URL(value);
    const allowedScheme = parsed.protocol === "http:" || parsed.protocol === "https:";
    return allowedScheme && !parsed.username && !parsed.password ? parsed : null;
  } catch (_) {
    return null;
  }
}

function setBusy(busy) {
  submitButton.disabled = busy;
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

function metadata(result) {
  const values = [];
  if (result.published_at) values.push(String(result.published_at));
  if (Array.isArray(result.providers) && result.providers.length === 1) {
    values.push(`Fuente: ${String(result.providers[0])}`);
  }
  return values.join(" · ");
}

function render() {
  resultsNode.replaceChildren();
  const start = state.page * PAGE_SIZE;
  const visible = state.results.slice(start, start + PAGE_SIZE);
  for (const result of visible) {
    const url = safeHttpUrl(result.canonical_url);
    if (!url || result.status !== "visible") continue;
    const fragment = template.content.cloneNode(true);
    const link = fragment.querySelector(".result-title");
    link.href = url.href;
    link.textContent = result.title || result.domain || url.hostname;
    fragment.querySelector(".result-url").textContent = result.domain || url.hostname;
    const snippet = fragment.querySelector(".result-snippet");
    snippet.textContent = result.snippet || "Sin descripción disponible.";
    const meta = fragment.querySelector(".result-meta");
    meta.textContent = metadata(result);
    meta.hidden = !meta.textContent;
    resultsNode.append(fragment);
  }
  const pages = Math.max(1, Math.ceil(state.results.length / PAGE_SIZE));
  paginationNode.hidden = pages <= 1;
  pageLabel.textContent = `Página ${state.page + 1} de ${pages}`;
  previousButton.disabled = state.page === 0;
  nextButton.disabled = state.page + 1 >= pages;
}

async function search(event) {
  event.preventDefault();
  if (!form.reportValidity()) return;
  if (state.controller) state.controller.abort();
  state.controller = new AbortController();
  state.results = [];
  state.page = 0;
  resultsNode.replaceChildren();
  paginationNode.hidden = true;
  setBusy(true);
  setStatus("Buscando…", "loading");
  try {
    const headers = {
      Accept: "application/json",
      "Content-Type": "application/json",
    };
    if (tokenInput.value) headers.Authorization = `Bearer ${tokenInput.value}`;
    const response = await fetch("/search", {
      method: "POST",
      headers,
      body: JSON.stringify({ q: queryText() }),
      credentials: "same-origin",
      signal: state.controller.signal,
    });
    if (response.status === 401) throw new Error("unauthorized");
    if (!response.ok) throw new Error("request_failed");
    const payload = await response.json();
    if (payload.schema_version !== "1" || !Array.isArray(payload.results)) {
      throw new Error("invalid_contract");
    }
    state.results = payload.results;
    render();
    if (state.results.length === 0) {
      setStatus("No se encontraron resultados.", "partial");
    } else if (payload.status === "partial_success") {
      setStatus(`${state.results.length} resultados; algunas fuentes no respondieron.`, "partial");
    } else {
      setStatus(`${state.results.length} resultados.`, "success");
    }
    resultsNode.focus({ preventScroll: true });
  } catch (error) {
    if (error.name !== "AbortError") {
      if (error.message === "unauthorized") {
        setStatus("El token de acceso falta o no es válido.", "error");
        tokenInput.focus();
      } else {
        setStatus("No fue posible completar la búsqueda. Intenta nuevamente.", "error");
      }
    }
  } finally {
    setBusy(false);
  }
}

form.addEventListener("submit", search);
window.addEventListener("pagehide", () => {
  tokenInput.value = "";
});
previousButton.addEventListener("click", () => {
  if (state.page > 0) state.page -= 1;
  render();
  resultsNode.focus();
});
nextButton.addEventListener("click", () => {
  if ((state.page + 1) * PAGE_SIZE < state.results.length) state.page += 1;
  render();
  resultsNode.focus();
});
