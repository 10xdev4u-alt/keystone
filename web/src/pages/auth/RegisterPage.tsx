import { useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useRegister } from "../../api/hooks";
import { ApiRequestError } from "../../api/client";
import { Button } from "../../components/Button/Button";
import { Input } from "../../components/Input/Input";
import { ErrorState } from "../../components/Status/Status";
import "./auth.css";

export function RegisterPage() {
  const navigate = useNavigate();
  const [email, setEmail] = useState("");
  const [firstName, setFirstName] = useState("");
  const [lastName, setLastName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const register = useRegister();

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (password !== confirm) {
      register.error; // placeholder to keep types simple
    }
    register.mutate(
      { email, password, first_name: firstName || undefined, last_name: lastName || undefined },
      {
        onSuccess: () => navigate("/verify", { state: { email } }),
      },
    );
  }

  const error =
    register.error instanceof ApiRequestError
      ? register.error.detail ?? register.error.message
      : null;

  const passwordMismatch = confirm.length > 0 && password !== confirm;

  return (
    <main className="auth-page">
      <form className="auth-card" onSubmit={onSubmit} noValidate>
        <header className="auth-card__header">
          <h1>Join Keystone</h1>
          <p>One account for courses, communities, and your network.</p>
        </header>

        <Input
          id="register-email"
          label="Email"
          type="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
        />
        <div className="auth-card__row">
          <Input
            id="register-first"
            label="First name"
            autoComplete="given-name"
            value={firstName}
            onChange={(e) => setFirstName(e.target.value)}
          />
          <Input
            id="register-last"
            label="Last name"
            autoComplete="family-name"
            value={lastName}
            onChange={(e) => setLastName(e.target.value)}
          />
        </div>
        <Input
          id="register-password"
          label="Password"
          type="password"
          autoComplete="new-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
        />
        <Input
          id="register-confirm"
          label="Confirm password"
          type="password"
          autoComplete="new-password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          aria-invalid={passwordMismatch || undefined}
          aria-describedby={passwordMismatch ? "register-confirm-error" : undefined}
          required
        />
        {passwordMismatch && (
          <p id="register-confirm-error" className="auth-card__error">
            Passwords do not match.
          </p>
        )}

        {error && <ErrorState title="Registration failed" message={error} />}

        <Button type="submit" loading={register.isPending} className="auth-card__submit">
          {register.isPending ? "Creating account…" : "Create account"}
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
          Already have an account? <Link to="/login">Sign in</Link>
        </p>
      </form>
    </main>
  );
}
