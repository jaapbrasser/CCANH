import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, unwrap } from "./client";
import type { components, operations } from "./schema";

export type User = components["schemas"]["User"];
export type UserListQuery = NonNullable<operations["list_users"]["parameters"]["query"]>;

const usersKey = ["users"] as const;

export function useUsers(query: UserListQuery) {
  return useQuery({
    queryKey: [...usersKey, query],
    queryFn: () => unwrap(api.GET("/api/v1/users", { params: { query } })),
    placeholderData: keepPreviousData,
  });
}

function useInvalidateUsers() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: usersKey });
}

export function useCreateUser() {
  const invalidate = useInvalidateUsers();
  return useMutation({
    mutationFn: (body: components["schemas"]["CreateUserRequest"]) =>
      unwrap(api.POST("/api/v1/users", { body })),
    onSuccess: invalidate,
  });
}

export function useUpdateUser() {
  const invalidate = useInvalidateUsers();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & components["schemas"]["UpdateUserRequest"]) =>
      unwrap(api.PATCH("/api/v1/users/{id}", { params: { path: { id } }, body })),
    onSuccess: invalidate,
  });
}

export function useDeleteUser() {
  const invalidate = useInvalidateUsers();
  return useMutation({
    mutationFn: (id: string) =>
      unwrap(api.DELETE("/api/v1/users/{id}", { params: { path: { id } } })),
    onSuccess: invalidate,
  });
}

export function useResetPassword() {
  return useMutation({
    mutationFn: ({ id, new_password }: { id: string; new_password: string }) =>
      unwrap(
        api.POST("/api/v1/users/{id}/password", {
          params: { path: { id } },
          body: { new_password },
        }),
      ),
  });
}
