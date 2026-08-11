import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LoginPage } from "./LoginPage";
import { RegisterPage } from "./RegisterPage";

// Top-level hoisted mock: the auth screens only call the mutation fn and read
// isPending/error, so a controllable stub is enough for unit tests.
const loginMutate = vi.hoisted(() => vi.fn());
const registerMutate = vi.hoisted(() => vi.fn());

vi.mock("../../api/hooks", () => ({
  useLogin: () => ({
    mutate: loginMutate,
    isPending: false,
    error: null,
  }),
  useRegister: () => ({
    mutate: registerMutate,
    isPending: false,
    error: null,
  }),
  useVerifyEmail: () => ({ mutate: vi.fn(), isPending: false, error: null }),
  useCurrentUser: () => ({ data: undefined, isPending: false, error: null }),
  usePosts: () => ({ data: undefined, isPending: false, error: null }),
  usePost: () => ({ data: undefined, isPending: false, error: null }),
  useCommunities: () => ({ data: undefined, isPending: false, error: null }),
  useCommunity: () => ({ data: undefined, isPending: false, error: null }),
  useCourses: () => ({ data: undefined, isPending: false, error: null }),
  useEvents: () => ({ data: undefined, isPending: false, error: null }),
  useOrgs: () => ({ data: undefined, isPending: false, error: null }),
  useProfile: () => ({ data: undefined, isPending: false, error: null }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage(node: React.ReactNode) {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/"]}>{node}</MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  loginMutate.mockReset();
  registerMutate.mockReset();
});

afterEach(() => {
  queryClient.clear();
});

describe("LoginPage", () => {
  it("renders the form fields and links", () => {
    renderPage(<LoginPage />);
    expect(screen.getByRole("heading", { name: "Welcome back" })).toBeInTheDocument();
    expect(screen.getByLabelText("Email")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Create an account" })).toHaveAttribute(
      "href",
      "/register",
    );
  });

  it("submits credentials to the login mutation", async () => {
    const user = userEvent.setup();
    renderPage(<LoginPage />);

    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "hunter2secret");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(loginMutate).toHaveBeenCalledWith({
      email: "ada@example.com",
      password: "hunter2secret",
    });
  });
});

describe("RegisterPage", () => {
  it("renders all fields", () => {
    renderPage(<RegisterPage />);
    expect(screen.getByRole("heading", { name: "Join Keystone" })).toBeInTheDocument();
    expect(screen.getByLabelText("Email")).toBeInTheDocument();
    expect(screen.getByLabelText("First name")).toBeInTheDocument();
    expect(screen.getByLabelText("Last name")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.getByLabelText("Confirm password")).toBeInTheDocument();
  });

  it("flags a password mismatch", async () => {
    const user = userEvent.setup();
    renderPage(<RegisterPage />);
    await user.type(screen.getByLabelText("Password"), "correcthorse");
    await user.type(screen.getByLabelText("Confirm password"), "batterystaple");
    expect(screen.getByText("Passwords do not match.")).toBeInTheDocument();
  });

  it("submits the signup payload", async () => {
    const user = userEvent.setup();
    renderPage(<RegisterPage />);
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("First name"), "Ada");
    await user.type(screen.getByLabelText("Last name"), "Lovelace");
    await user.type(screen.getByLabelText("Password"), "correcthorse");
    await user.type(screen.getByLabelText("Confirm password"), "correcthorse");
    await user.click(screen.getByRole("button", { name: "Create account" }));

    expect(registerMutate).toHaveBeenCalledWith(
      {
        email: "ada@example.com",
        password: "correcthorse",
        first_name: "Ada",
        last_name: "Lovelace",
      },
      expect.anything(),
    );
  });
});
