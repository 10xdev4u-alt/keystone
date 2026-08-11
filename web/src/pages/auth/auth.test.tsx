import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LoginPage } from "./LoginPage";
import { RegisterPage } from "./RegisterPage";
import { ForgotPasswordPage } from "./ForgotPasswordPage";
import { ResetPasswordPage } from "./ResetPasswordPage";

// Top-level hoisted mock: the auth screens only call the mutation fn and read
// isPending/error, so a controllable stub is enough for unit tests.
const loginMutate = vi.hoisted(() => vi.fn());
const registerMutate = vi.hoisted(() => vi.fn());
const forgotMutate = vi.hoisted(() => vi.fn());
const resetMutate = vi.hoisted(() => vi.fn());

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
  useForgotPassword: () => ({
    mutate: forgotMutate,
    isPending: false,
    error: null,
  }),
  useResetPassword: () => ({
    mutate: resetMutate,
    isPending: false,
    error: null,
  }),
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
  forgotMutate.mockReset();
  resetMutate.mockReset();
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

describe("ForgotPasswordPage", () => {
  it("submits the email and shows the sent state", async () => {
    forgotMutate.mockImplementation((_args, opts) => opts?.onSuccess?.());
    renderPage(<ForgotPasswordPage />);
    await userEvent.type(screen.getByLabelText("Email"), "ada@example.com");
    await userEvent.click(screen.getByRole("button", { name: "Send reset link" }));
    expect(forgotMutate).toHaveBeenCalledWith(
      { email: "ada@example.com" },
      expect.anything(),
    );
    expect(screen.getByText(/reset token is on the way/i)).toBeInTheDocument();
  });
});

describe("ResetPasswordPage", () => {
  it("submits matching passwords with the token", async () => {
    renderPage(<ResetPasswordPage />);
    await userEvent.type(screen.getByLabelText("Email"), "ada@example.com");
    await userEvent.type(screen.getByLabelText("New password"), "newpass123");
    await userEvent.type(screen.getByLabelText("Confirm new password"), "newpass123");
    await userEvent.click(screen.getByRole("button", { name: "Set new password" }));
    expect(resetMutate).toHaveBeenCalledWith({
      email: "ada@example.com",
      token: "",
      new_password: "newpass123",
    });
  });

  it("blocks submission when passwords mismatch", async () => {
    renderPage(<ResetPasswordPage />);
    await userEvent.type(screen.getByLabelText("Email"), "ada@example.com");
    await userEvent.type(screen.getByLabelText("New password"), "newpass123");
    await userEvent.type(screen.getByLabelText("Confirm new password"), "different");
    expect(
      screen.getByText(/passwords don't match/i),
    ).toBeInTheDocument();
    const button = screen.getByRole("button", { name: "Set new password" });
    expect(button).toBeDisabled();
  });
});
