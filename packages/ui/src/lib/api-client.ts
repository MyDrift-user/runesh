"use client";

// Same-origin by default (Caddy proxies /api/* to the backend).
let API_BASE = "";
let apiBaseSet = false;

/** Override the API base URL. Can only be called once. */
export function setApiBase(base: string) {
  if (apiBaseSet) {
    console.warn("setApiBase() called more than once -- ignoring");
    return;
  }
  API_BASE = base;
  apiBaseSet = true;
}

/** Read the CSRF token from the __Host-csrf cookie (production) or csrf cookie (dev). */
function getCsrfToken(): string | null {
  if (typeof document === "undefined") return null;
  const cookies = document.cookie.split(";").map((c) => c.trim());
  // Try production cookie first, then dev-mode cookie
  const match =
    cookies.find((c) => c.startsWith("__Host-csrf=")) ??
    cookies.find((c) => c.startsWith("csrf="));
  return match?.split("=")[1] ?? null;
}

// Deduplicate concurrent refresh attempts
let refreshPromise: Promise<boolean> | null = null;

async function refreshSession(): Promise<boolean> {
  if (refreshPromise) return refreshPromise;
  refreshPromise = (async () => {
    try {
      const res = await fetch(`${API_BASE}/api/auth/refresh`, {
        method: "POST",
        credentials: "include",
        headers: { "X-CSRF-Token": getCsrfToken() ?? "" },
      });
      return res.ok;
    } catch {
      return false;
    }
  })();
  try {
    return await refreshPromise;
  } finally {
    refreshPromise = null;
  }
}

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const method = options?.method?.toUpperCase() ?? "GET";
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(options?.headers as Record<string, string>),
  };

  // Add CSRF token for state-changing methods
  if (["POST", "PUT", "PATCH", "DELETE"].includes(method)) {
    const csrf = getCsrfToken();
    if (csrf) headers["X-CSRF-Token"] = csrf;
  }

  const res = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers,
    credentials: "include",
    cache: "no-store" as RequestCache,
  });

  // On 401, try refreshing the session once (deduplicated)
  if (res.status === 401) {
    const refreshed = await refreshSession();
    if (refreshed) {
      // Re-read CSRF token after refresh (it may have rotated)
      const freshCsrf = getCsrfToken();
      if (freshCsrf && ["POST", "PUT", "PATCH", "DELETE"].includes(method)) {
        headers["X-CSRF-Token"] = freshCsrf;
      }

      const retryRes = await fetch(`${API_BASE}${path}`, {
        ...options,
        headers,
        credentials: "include",
        cache: "no-store" as RequestCache,
      });

      if (retryRes.ok) {
        if (retryRes.status === 204) return undefined as unknown as T;
        return retryRes.json();
      }
    }

    // Refresh failed - redirect to login
    if (typeof window !== "undefined") {
      window.location.href = "/login";
    }
    throw new Error("Session expired");
  }

  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error || "Request failed");
  }
  if (res.status === 204) return undefined as unknown as T;
  return res.json();
}

/** Shared API client with cookie-based auth, CSRF protection, and 401 auto-refresh. */
export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "POST", body: body !== undefined ? JSON.stringify(body) : undefined }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "PUT", body: body !== undefined ? JSON.stringify(body) : undefined }),
  patch: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "PATCH", body: body !== undefined ? JSON.stringify(body) : undefined }),
  delete: <T>(path: string) => request<T>(path, { method: "DELETE" }),
};

/**
 * Upload a file with optional progress tracking.
 */
export async function uploadFile(
  path: string,
  formData: FormData,
  onProgress?: (pct: number) => void,
): Promise<unknown> {
  if (onProgress) {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open("POST", `${API_BASE}${path}`);
      xhr.withCredentials = true;
      const csrf = getCsrfToken();
      if (csrf) xhr.setRequestHeader("X-CSRF-Token", csrf);
      xhr.timeout = 3_600_000;

      xhr.upload.addEventListener("progress", (e) => {
        if (e.lengthComputable) onProgress(Math.round((e.loaded / e.total) * 100));
      });
      xhr.addEventListener("load", () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          try { resolve(JSON.parse(xhr.responseText)); } catch { resolve(undefined); }
        } else {
          reject(new Error("Upload failed"));
        }
      });
      xhr.addEventListener("error", () => reject(new Error("Upload failed")));
      xhr.send(formData);
    });
  }

  const headers: Record<string, string> = {};
  const csrf = getCsrfToken();
  if (csrf) headers["X-CSRF-Token"] = csrf;

  const res = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers,
    body: formData,
    credentials: "include",
  });
  if (!res.ok) throw new Error("Upload failed");
  if (res.status === 204) return undefined;
  return res.json();
}
