/**
 * PKCE (Proof Key for Code Exchange) utilities for OIDC frontend flows.
 */

function base64urlEncode(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function generateCodeVerifier(): string {
  const array = new Uint8Array(64);
  crypto.getRandomValues(array);
  return base64urlEncode(array.buffer);
}

export async function generateCodeChallenge(verifier: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(verifier);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return base64urlEncode(hash);
}

export function generateState(): string {
  const array = new Uint8Array(32);
  crypto.getRandomValues(array);
  return base64urlEncode(array.buffer);
}

export interface AuthConfig {
  authorizationEndpoint: string;
  clientId: string;
  redirectUri: string;
  scope: string;
}

export function buildAuthUrl(
  config: AuthConfig,
  codeChallenge: string,
  state: string,
): string {
  const params = new URLSearchParams({
    client_id: config.clientId,
    response_type: "code",
    redirect_uri: config.redirectUri,
    scope: config.scope,
    code_challenge: codeChallenge,
    code_challenge_method: "S256",
    state,
    response_mode: "query",
  });
  return `${config.authorizationEndpoint}?${params.toString()}`;
}

export interface OidcPending {
  verifier: string;
  state: string;
}

const PENDING_KEY = "oidc_pending";
const PENDING_TTL_MS = 10 * 60 * 1000; // 10 minutes

interface StoredPending extends OidcPending {
  ts: number;
}

// Use sessionStorage for PKCE verifier (scoped to tab, not accessible from other windows)
export function storePending(pending: OidcPending): void {
  const data: StoredPending = { ...pending, ts: Date.now() };
  sessionStorage.setItem(PENDING_KEY, JSON.stringify(data));
}

export function retrievePending(): OidcPending | null {
  const raw = sessionStorage.getItem(PENDING_KEY);
  if (!raw) return null;
  sessionStorage.removeItem(PENDING_KEY);
  try {
    const data: StoredPending = JSON.parse(raw);
    if (data.ts && Date.now() - data.ts > PENDING_TTL_MS) return null;
    return { verifier: data.verifier, state: data.state };
  } catch {
    return null;
  }
}
