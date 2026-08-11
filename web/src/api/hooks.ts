//! TanStack Query hooks over the generated, typed API client.
//!
//! Every hook is fully typed from the OpenAPI spec — the backend's schema is
//! the single source of truth. Adding a screen never requires writing a fetch
//! or a hand-rolled type.

import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationOptions,
  type UseQueryOptions,
} from "@tanstack/react-query";
import { ApiRequestError, client, setTokens } from "./client";
import type { components, operations } from "./generated";

type TokenResponse = components["schemas"]["TokenResponse"];
type UserView = components["schemas"]["UserView"];
type RegisterRequest = components["schemas"]["SignupRequest"];
type LoginRequest = components["schemas"]["LoginRequest"];
type PostListPage = components["schemas"]["PostListPage"];

// ── Content ──────────────────────────────────────────────────────────────────

/** The homepage feed — newest posts first, keyset-paginated. */
export function usePosts(
  params: { kind?: string; limit?: number; before?: string } = {},
  options?: UseQueryOptions<PostListPage, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["posts", params],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/posts", {
        params: { query: params },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty posts response");
      return data;
    },
    staleTime: 15_000,
    ...options,
  });
}

// ── Auth ────────────────────────────────────────────────────────────────────

export function useCurrentUser(options?: UseQueryOptions<UserView, ApiRequestError>) {
  return useQuery({
    queryKey: ["auth", "me"],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/auth/me");
      if (error) throw error;
      if (!data) throw new Error("Empty /auth/me response");
      return data;
    },
    retry: false,
    staleTime: 60_000,
    ...options,
  });
}

/** Swap the in-memory tokens after any successful authenticate/refresh. */
function applyAuth(body: TokenResponse): TokenResponse {
  setTokens(body.access_token, body.csrf_token ?? null);
  return body;
}

export function useLogin(
  options?: UseMutationOptions<TokenResponse, ApiRequestError, LoginRequest>,
) {
  const qc = useQueryClient();
  const { onSuccess: userOnSuccess, ...rest } = options ?? {};
  return useMutation({
    ...rest,
    mutationFn: async (req) => {
      const { data, error } = await client.POST("/api/v1/auth/login", { body: req });
      if (error) throw error;
      if (!data) throw new Error("Empty login response");
      return applyAuth(data);
    },
    onSuccess: (data, vars, ctx, mutation) => {
      qc.setQueryData(["auth", "me"], data.user);
      userOnSuccess?.(data, vars, ctx, mutation);
    },
  });
}

export function useRegister(
  options?: UseMutationOptions<TokenResponse, ApiRequestError, RegisterRequest>,
) {
  const qc = useQueryClient();
  const { onSuccess: userOnSuccess, ...rest } = options ?? {};
  return useMutation({
    ...rest,
    mutationFn: async (req) => {
      const { data, error } = await client.POST("/api/v1/auth/register", { body: req });
      if (error) throw error;
      if (!data) throw new Error("Empty register response");
      return applyAuth(data);
    },
    onSuccess: (data, vars, ctx, mutation) => {
      qc.setQueryData(["auth", "me"], data.user);
      userOnSuccess?.(data, vars, ctx, mutation);
    },
  });
}

export function useVerifyEmail(
  options?: UseMutationOptions<void, ApiRequestError, { token: string }>,
) {
  return useMutation({
    mutationFn: async ({ token }) => {
      const { error } = await client.POST("/api/v1/auth/verify-email", {
        body: { token },
      });
      if (error) throw error;
    },
    ...options,
  });
}

export function useLogout(options?: UseMutationOptions<void, ApiRequestError, void>) {
  const qc = useQueryClient();
  const { onSuccess: userOnSuccess, ...rest } = options ?? {};
  return useMutation({
    ...rest,
    mutationFn: async () => {
      const { error } = await client.POST("/api/v1/auth/logout");
      if (error) throw error;
      setTokens(null, null);
    },
    onSuccess: (data, vars, ctx, mutation) => {
      qc.clear();
      userOnSuccess?.(data, vars, ctx, mutation);
    },
  });
}

export function usePost(
  id: string,
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["posts", id],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/posts/{id}", {
        params: { path: { id } },
      });
      if (error) throw error;
      return data;
    },
    enabled: Boolean(id),
    ...options,
  });
}

// ── Social ──────────────────────────────────────────────────────────────────

export function useCommunities(
  query: operations["list_communities"]["parameters"]["query"] = {},
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["communities", query],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/communities", {
        params: { query },
      });
      if (error) throw error;
      return data;
    },
    ...options,
  });
}

export function useCommunity(
  slug: string,
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["communities", slug],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/communities/{slug}", {
        params: { path: { slug } },
      });
      if (error) throw error;
      return data;
    },
    enabled: Boolean(slug),
    ...options,
  });
}

// ── Learning ────────────────────────────────────────────────────────────────

export function useCourses(
  query: operations["list_courses"]["parameters"]["query"] = {},
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["courses", query],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/courses", {
        params: { query },
      });
      if (error) throw error;
      return data;
    },
    ...options,
  });
}

export function useEvents(
  query: operations["list_events"]["parameters"]["query"] = {},
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["events", query],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/events", {
        params: { query },
      });
      if (error) throw error;
      return data;
    },
    ...options,
  });
}

// ── Network ─────────────────────────────────────────────────────────────────

export function useOrgs(
  query: operations["list_orgs"]["parameters"]["query"] = {},
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["orgs", query],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/orgs", { params: { query } });
      if (error) throw error;
      return data;
    },
    ...options,
  });
}

export function useProfile(
  userId: string,
  options?: UseQueryOptions<unknown, ApiRequestError>,
) {
  return useQuery({
    queryKey: ["profiles", userId],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/users/{user_id}/profile", {
        params: { path: { user_id: userId } },
      });
      if (error) throw error;
      return data;
    },
    enabled: Boolean(userId),
    ...options,
  });
}
