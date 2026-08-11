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
type PostDetailResponse = components["schemas"]["PostDetailResponse"];
type CommentList = components["schemas"]["CommentList"];
type CommentResponse = components["schemas"]["CommentResponse"];
type CreateCommentRequest = components["schemas"]["CreateCommentRequest"];
type CommunityList = components["schemas"]["CommunityList"];
type CommunityDetailResponse = components["schemas"]["CommunityDetailResponse"];
type MemberList = components["schemas"]["MemberList"];
type CommunityPostList = components["schemas"]["CommunityPostList"];
type EventList = components["schemas"]["EventList"];

/**
 * Caller-facing options for query wrapper hooks. The hook owns `queryKey` and
 * `queryFn`; callers may pass any other option (staleTime, enabled, select,
 * placeholderData, …) without satisfying the queryKey field.
 */
type QueryOptions<T> = Omit<UseQueryOptions<T, ApiRequestError>, "queryKey" | "queryFn">;

// ── Content ──────────────────────────────────────────────────────────────────

/** The homepage feed — newest posts first, keyset-paginated. */
export function usePosts(
  params: { kind?: string; limit?: number; before?: string } = {},
  options?: QueryOptions<PostListPage>,
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

export function useCurrentUser(options?: QueryOptions<UserView>) {
  return useQuery({
    queryKey: ["auth", "me"],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/auth/me");
      if (error) throw error;
      if (!data?.user) throw new Error("Empty /auth/me response");
      return data.user;
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

/** Full post reader view — slug or UUID. */
export function usePost(
  id: string,
  options?: QueryOptions<PostDetailResponse>,
) {
  return useQuery({
    queryKey: ["posts", id],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/posts/{id}", {
        params: { path: { id } },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty post response");
      return data;
    },
    enabled: Boolean(id),
    staleTime: 30_000,
    ...options,
  });
}

/** Comment thread for a post. */
export function useComments(
  postId: string,
  options?: QueryOptions<CommentList>,
) {
  return useQuery({
    queryKey: ["comments", postId],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/posts/{id}/comments", {
        params: { path: { id: postId } },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty comments response");
      return data;
    },
    enabled: Boolean(postId),
    ...options,
  });
}

export function useCreateComment(
  postId: string,
  options?: UseMutationOptions<CommentResponse, ApiRequestError, CreateCommentRequest>,
) {
  const qc = useQueryClient();
  const { onSuccess: userOnSuccess, ...rest } = options ?? {};
  return useMutation({
    ...rest,
    mutationFn: async (req) => {
      const { data, error } = await client.POST("/api/v1/posts/{id}/comments", {
        params: { path: { id: postId } },
        body: req,
      });
      if (error) throw error;
      if (!data) throw new Error("Empty comment response");
      return data;
    },
    onSuccess: (data, vars, ctx, mutation) => {
      qc.invalidateQueries({ queryKey: ["comments", postId] });
      userOnSuccess?.(data, vars, ctx, mutation);
    },
  });
}

// ── Social ──────────────────────────────────────────────────────────────────

export function useCommunities(
  query: operations["list_communities"]["parameters"]["query"] = {},
  options?: QueryOptions<CommunityList>,
) {
  return useQuery({
    queryKey: ["communities", query],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/communities", {
        params: { query },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty communities response");
      return data;
    },
    staleTime: 30_000,
    ...options,
  });
}

export function useCommunity(
  slug: string,
  options?: QueryOptions<CommunityDetailResponse>,
) {
  return useQuery({
    queryKey: ["communities", slug],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/communities/{slug}", {
        params: { path: { slug } },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty community response");
      return data;
    },
    enabled: Boolean(slug),
    staleTime: 30_000,
    ...options,
  });
}

export function useCommunityMembers(
  slug: string,
  options?: QueryOptions<MemberList>,
) {
  return useQuery({
    queryKey: ["communities", slug, "members"],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/communities/{slug}/members", {
        params: { path: { slug } },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty members response");
      return data;
    },
    enabled: Boolean(slug),
    staleTime: 60_000,
    ...options,
  });
}

export function useCommunityPosts(
  slug: string,
  options?: QueryOptions<CommunityPostList>,
) {
  return useQuery({
    queryKey: ["communities", slug, "posts"],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/communities/{slug}/posts", {
        params: { path: { slug } },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty community posts response");
      return data;
    },
    enabled: Boolean(slug),
    ...options,
  });
}

export function useJoinCommunity(
  slug: string,
  options?: UseMutationOptions<void, ApiRequestError, void>,
) {
  const qc = useQueryClient();
  const { onSuccess: userOnSuccess, ...rest } = options ?? {};
  return useMutation({
    ...rest,
    mutationFn: async () => {
      const { error } = await client.POST("/api/v1/communities/{slug}/join", {
        params: { path: { slug } },
      });
      if (error) throw error;
    },
    onSuccess: (data, vars, ctx, mutation) => {
      qc.invalidateQueries({ queryKey: ["communities", slug, "members"] });
      userOnSuccess?.(data, vars, ctx, mutation);
    },
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
  options?: QueryOptions<EventList>,
) {
  return useQuery({
    queryKey: ["events", query],
    queryFn: async () => {
      const { data, error } = await client.GET("/api/v1/events", {
        params: { query },
      });
      if (error) throw error;
      if (!data) throw new Error("Empty events response");
      return data;
    },
    staleTime: 30_000,
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
