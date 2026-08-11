import { useState, type FormEvent } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { useLogin } from "../../api/hooks";
import { ApiRequestError } from "../../api/client";
import { Button } from "../../components/Button/Button";
import { Input } from "../../components/Input/Input";
import { ErrorState } from "../../components/Status/Status";
import "./auth.css";

export function LoginPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? "/";
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const login = useLogin({
    onSuccess: () => navigate(from, { replace: true }),
  });

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    login.mutate({ email, password });
  }

  const error =
    login.error instanceof ApiRequestError
      ? login.error.detail ?? login.error.message
      : null;

  return (
    <main className="auth-page">
      <form className="auth-card" onSubmit={onSubmit} noValidate>
        <header className="auth-card__header">
          <h1>Welcome back</h1>
          <p>Sign in to continue to Keystone.</p>
        </header>

        <Input
          id="login-email"
          label="Email"
          type="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
        />
        <Input
          id="login-password"
          label="Password"
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
        />

        {error && <ErrorState title="Sign-in failed" message={error} />}

        <p className="auth-card__forgot">
          <Link to="/forgot-password">Forgot password?</Link>
        </p>

        <Button type="submit" loading={login.isPending} className="auth-card__submit">
          {login.isPending ? "Signing in…" : "Sign in"}
        </Button>

        <div className="auth-card__divider">
          <span>or</span>
        </div>

        <a
          href="/api/v1/auth/oauth/google/start"
          className="auth-card__oauth"
          aria-label="Continue with Google"
        >
          <svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18">
            <path
              fill="#4285F4"
              d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.27-4.74 3.27-8.1z"
            />
            <path
              fill="#34A853"
              d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84A11 11 0 0 0 12 23z"
            />
            <path
              fill="#FBBC05"
              d="M5.84 14.1a6.6 6.6 0 0 1 0-4.2V7.06H2.18a11 11 0 0 0 0 9.88l3.66-2.84z"
            />
            <path
              fill="#EA4335"
              d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15A11 11 0 0 0 2.18 7.06l3.66 2.84c.87-2.6 3.3-4.52 6.16-4.52z"
            />
          </svg>
          Continue with Google
        </a>

        <p className="auth-card__switch">
          New here? <Link to="/register">Create an account</Link>
        </p>
      </form>
    </main>
  );
}
