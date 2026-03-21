"use client";

import { createContext, useContext, useEffect, useState, useCallback, useRef } from "react";
import { getAccessToken, getStoredUser, storeTokens, clearTokens, isTokenExpiringSoon, type StoredUser } from "@/lib/token-store";
import { api } from "@/lib/api-client";

interface AuthContextValue {
  user: StoredUser | null;
  isLoading: boolean;
  isAuthenticated: boolean;
  /** Call after successful login to store tokens and set user */
  setSession: (accessToken: string, refreshToken: string, expiresIn: number, user: StoredUser) => void;
  /** Clear session and redirect to login */
  logout: () => void;
  /** Get current access token (refreshes if needed) */
  getToken: () => string | null;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function useAuth() {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within an AuthProvider");
  return ctx;
}

interface AuthProviderProps {
  children: React.ReactNode;
  /** API endpoint to validate token and get current user (default: "/api/auth/me") */
  meEndpoint?: string;
  /** Where to redirect on logout (default: "/login") */
  loginPath?: string;
  /** Refresh interval in ms (default: 780000 = 13 minutes) */
  refreshInterval?: number;
  /** Paths that don't require auth (checked with startsWith) */
  publicPaths?: string[];
}

export function AuthProvider({
  children,
  meEndpoint = "/api/auth/me",
  loginPath = "/login",
  refreshInterval = 780_000,
  publicPaths = ["/login", "/auth/callback", "/setup"],
}: AuthProviderProps) {
  const [user, setUser] = useState<StoredUser | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const refreshTimer = useRef<ReturnType<typeof setInterval>>();

  const logout = useCallback(() => {
    clearTokens();
    setUser(null);
    if (typeof window !== "undefined") {
      window.location.href = loginPath;
    }
  }, [loginPath]);

  const setSession = useCallback((accessToken: string, refreshToken: string, expiresIn: number, userData: StoredUser) => {
    storeTokens(accessToken, refreshToken, expiresIn, userData);
    setUser(userData);
  }, []);

  const getToken = useCallback(() => getAccessToken(), []);

  // Validate stored session on mount
  useEffect(() => {
    const init = async () => {
      const token = getAccessToken();
      const stored = getStoredUser();

      if (!token) {
        setIsLoading(false);
        return;
      }

      // Try to validate with the server
      try {
        const me = await api.get<StoredUser>(meEndpoint);
        setUser(me);
      } catch {
        // Server unreachable or token invalid - use stored user if token not expired
        if (stored && !isTokenExpiringSoon()) {
          setUser(stored);
        } else {
          clearTokens();
        }
      }
      setIsLoading(false);
    };

    init();
  }, [meEndpoint]);

  // Schedule periodic token refresh
  useEffect(() => {
    if (!user) return;

    refreshTimer.current = setInterval(async () => {
      if (isTokenExpiringSoon()) {
        try {
          await api.get(meEndpoint); // triggers auto-refresh via api-client
        } catch {
          logout();
        }
      }
    }, refreshInterval);

    return () => {
      if (refreshTimer.current) clearInterval(refreshTimer.current);
    };
  }, [user, meEndpoint, refreshInterval, logout]);

  return (
    <AuthContext.Provider value={{
      user,
      isLoading,
      isAuthenticated: !!user,
      setSession,
      logout,
      getToken,
    }}>
      {children}
    </AuthContext.Provider>
  );
}
