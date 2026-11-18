import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, api, unwrap } from "./client";
import type { components } from "./schema";

export type Me = components["schemas"]["Me"];
export type Role = components["schemas"]["Role"];

export const meKey = ["auth", "me"] as const;

/** The signed-in user, or `null` when signed out. */
export function useMe() {
  return useQuery({
    queryKey: meKey,
    queryFn: async (): Promise<Me | null> => {
      try {
        return await unwrap(api.GET("/api/v1/auth/me"));
      } catch (err) {
        if (err instanceof ApiError && err.status === 401) return null;
        throw err;
      }
    },
    staleTime: 60_000,
    retry: false,
  });
}

export function useLogin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { username: string; password: string }) =>
      unwrap(api.POST("/api/v1/auth/login", { body })),
    onSuccess: (me) => {
      qc.clear();
      qc.setQueryData(meKey, me);
    },
  });
}

export function useLogout() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/auth/logout")),
    onSettled: () => {
      qc.clear();
      qc.setQueryData(meKey, null);
    },
  });
}

export function useChangePassword() {
  return useMutation({
    mutationFn: (body: { current_password: string; new_password: string }) =>
      unwrap(api.POST("/api/v1/auth/password", { body })),
  });
}

export function useSetupStatus() {
  return useQuery({
    queryKey: ["setup"],
    queryFn: () => unwrap(api.GET("/api/v1/setup")),
    retry: false,
  });
}

export function useCompleteSetup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { token: string; username: string; password: string }) =>
      unwrap(api.POST("/api/v1/setup", { body })),
    onSuccess: (me) => {
      qc.clear();
      qc.setQueryData(meKey, me);
      qc.setQueryData(["setup"], { setup_required: false });
    },
  });
}

export const MIN_PASSWORD_LENGTH = 12;
