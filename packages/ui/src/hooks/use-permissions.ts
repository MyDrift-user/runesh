"use client";

import { useMemo } from "react";
import { useAuth } from "@mydrift-user/runesh-ui/src/components/providers/auth-provider";

export function usePermissions() {
  const { user } = useAuth();

  return useMemo(() => ({
    hasPermission: (perm: string) =>
      user?.role === "admin" || (user?.permissions?.includes(perm) ?? false),

    hasAnyPermission: (...perms: string[]) =>
      user?.role === "admin" || perms.some((p) => user?.permissions?.includes(p) ?? false),

    isAdmin: user?.role === "admin",
  }), [user]);
}
