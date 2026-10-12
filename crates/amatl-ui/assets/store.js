"use strict";

// Simple reactive store for UI state.
const createStore = (initialState) => {
  let state = { ...initialState };
  const listeners = [];

  return {
    get: () => state,
    update: (partial) => {
      state = { ...state, ...partial };
      listeners.forEach((l) => l(state));
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
