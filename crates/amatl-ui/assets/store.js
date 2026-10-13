"use strict";

// Simple reactive store for UI state.
//
// The store is intentionally coarse-grained: `update` fans out to every
// subscriber and the app-level subscriber re-renders the whole result list.
// `updateSilent` exists for *transient UI* state that must not pay that full
// render (e.g. the Deep target checkboxes below the results): it keeps the
// value in `get()` so `run()` can still read it, but makes no notification, so
// toggling a checkbox neither rebuilds the list nor drops keyboard focus.
// Only use it for ephemeral state that drives no rendering by itself.
const createStore = (initialState) => {
  let state = { ...initialState };
  const listeners = [];

  return {
    get: () => state,
    update: (partial) => {
      state = { ...state, ...partial };
      listeners.forEach((l) => l(state));
    },
    // Mutates state without notifying subscribers (transient UI state only).
    updateSilent: (partial) => {
      state = { ...state, ...partial };
    },
    subscribe: (listener) => {
      listeners.push(listener);
      return () => {
        const index = listeners.indexOf(listener);
        if (index > -1) listeners.splice(index, 1);
      };
    },
  };
};

globalThis.store = createStore({
  items: [],
  mode: "search",
  page: 0,
  totalPages: 1,
  totalResults: 0,
  controller: null,
  answer: null,
  selectedTargets: [],
});
