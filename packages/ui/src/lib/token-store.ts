/**
 * Token storage for JWT-based authentication.
 *
 * Stores access token, refresh token, expiry, and user info in localStorage.
 * Configure the key prefix to avoid collisions between projects:
 *
 *   import { setTokenPrefix } from "@runesh/ui/lib/token-store";
 *   setTokenPrefix("myapp");  // keys become myapp_access_token, etc.
 */

let PREFIX = "app";

export function setTokenPrefix(prefix: string) {
  PREFIX = prefix;
}

function key(name: string) {
  return `${PREFIX}_${name}`;
}

export interface StoredUser {
  id: string;
  name: string;
  email: string;
  role: string;
  avatar_url: string | null;
  permissions?: string[];
}

export function getAccessToken(): string | null {
  if (typeof window === "undefined") return null;
  return localStorage.getItem(key("access_token"));
}

export function getRefreshToken(): string | null {
  if (typeof window === "undefined") return null;
  return localStorage.getItem(key("refresh_token"));
}

export function getStoredUser(): StoredUser | null {
  if (typeof window === "undefined") return null;
  const raw = localStorage.getItem(key("user"));
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

export function storeTokens(
  accessToken: string,
  refreshToken: string,
  expiresIn: number,
  user?: StoredUser,
): void {
  localStorage.setItem(key("access_token"), accessToken);
  localStorage.setItem(key("refresh_token"), refreshToken);
  localStorage.setItem(key("token_expiry"), String(Date.now() + expiresIn * 1000));
  if (user) {
    localStorage.setItem(key("user"), JSON.stringify(user));
  }
}

export function clearTokens(): void {
  localStorage.removeItem(key("access_token"));
  localStorage.removeItem(key("refresh_token"));
  localStorage.removeItem(key("token_expiry"));
  localStorage.removeItem(key("user"));
}

/** Returns true if the token will expire within 2 minutes. */
export function isTokenExpiringSoon(): boolean {
  if (typeof window === "undefined") return true;
  const expiry = localStorage.getItem(key("token_expiry"));
  if (!expiry) return true;
  return Date.now() > Number(expiry) - 120_000;
}
