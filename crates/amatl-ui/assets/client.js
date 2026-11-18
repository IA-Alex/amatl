"use strict";

// Centralized API client to manage transport, authentication, and error handling.
const client = {
  async request(path, options = {}) {
    const headers = options.headers || {};
    // Inject authorization
    const authHeaders = typeof globalThis.authHeaders === "function" ? globalThis.authHeaders() : {};
    
    const response = await fetch(path, {
      ...options,
      headers: {
        "Content-Type": "application/json",
        ...authHeaders,
        ...headers,
      },
    });

    if (!response.ok) {
      let payload = null;
      try {
        payload = await response.json();
      } catch (_) {}
      const error = new Error(
        payload?.error?.message || `API Error: ${response.status} ${response.statusText}`,
      );
      error.code = typeof payload?.error?.code === "string" ? payload.error.code : null;
      error.status = response.status;
      throw error;
    }

    return await response.json();
  },

  async get(path) {
    return this.request(path, { method: "GET" });
  },

  async post(path, payload, options = {}) {
    return this.request(path, {
      method: "POST",
      body: JSON.stringify(payload),
      ...options,
    });
  },

  async patch(path, payload, options = {}) {
    return this.request(path, {
      method: "PATCH",
      body: JSON.stringify(payload),
      ...options,
    });
  },

  async delete(path, options = {}) {
    return this.request(path, { method: "DELETE", ...options });
  },
};

globalThis.api = client;
