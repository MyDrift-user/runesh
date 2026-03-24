"use client";

import { createContext, useContext, useEffect, useState, useCallback, useRef } from "react";
import { api } from "../../lib/api-client";

export interface AuthUser {
  id: string;
  name: string;
  email: string;
  role: string;
  avatar_url: string | null;
  permissions?: string[];
}

interface AuthContextValue {
  user: AuthUser | null;
  isLoading: boolean;
  isAuthenticated: boolean;
  /** Redirect to OIDC login */
  login: () => void;
  /** Clear session and redirect to login page */
  logout: () => void;
  /** Refresh current user data from server */
  refreshUser: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function useAuth() {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within an AuthProvider");
  return ctx;
}

interface AuthProviderProps {
  children: React.ReactNode;
  /** Where to redirect on logout (default: "/login") */
  loginPath?: string;
  /** How often to refresh the session in ms (default: 780000 = 13 min) */
  refreshInterval?: number;
}

export function AuthProvider({
  children,
  loginPath = "/login",
  refreshInterval = 780_000,
}: AuthProviderProps) {
  const [user, setUser] = useState<AuthUser | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const refreshTimer = useRef<ReturnType<typeof setInterval>>(undefined);

  const fetchUser = useCallback(async () => {
    try {
      const me = await api.get<AuthUser>("/api/auth/me");
      setUser(me);
      return true;
    } catch {
      setUser(null);
      return false;
    }
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.post("/api/auth/logout");
    } catch {
      // best-effort
    }
    setUser(null);
    if (typeof window !== "undefined") {
      // Validate loginPath is a safe relative path (prevent open redirect)
      const safePath = loginPath.startsWith("/") && !loginPath.startsWith("//") && !loginPath.includes(":")
        ? loginPath : "/login";
      window.location.href = safePath;
    }
  }, [loginPath]);

  const login = useCallback(() => {
    // Redirect to OIDC login start - backend returns the auth URL
    window.location.href = "/api/auth/login/start";
  }, []);

  const refreshUser = useCallback(async () => {
    await fetchUser();
  }, [fetchUser]);

  // Check session on mount
  useEffect(() => {
    fetchUser().finally(() => setIsLoading(false));
  }, [fetchUser]);

  // Periodic session refresh (keeps cookies alive)
  useEffect(() => {
    if (!user) return;

    refreshTimer.current = setInterval(async () => {
      try {
        await api.post("/api/auth/refresh");
        await fetchUser();
      } catch {
        setUser(null);
      }
    }, refreshInterval);

    return () => {
      if (refreshTimer.current) clearInterval(refreshTimer.current);
    };
  }, [user, refreshInterval, fetchUser]);

  return (
    <AuthContext.Provider value={{
      user,
      isLoading,
      isAuthenticated: !!user,
      login,
      logout,
      refreshUser,
    }}>
      {children}
    </AuthContext.Provider>
  );
}
