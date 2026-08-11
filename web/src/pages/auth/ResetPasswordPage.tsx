import { useState, type FormEvent } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { useResetPassword } from "../../api/hooks";
import { ApiRequestError } from "../../api/client";
import { Button } from "../../components/Button/Button";
import { Input } from "../../components/Input/Input";
import { ErrorState } from "../../components/Status/Status";
import "./auth.css";

export function ResetPasswordPage() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const reset = useResetPassword({
    onSuccess: () => navigate("/login", { replace: true }),
  });

  const token = params.get("token") ?? "";

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (password !== confirm) return;
    reset.mutate({ email, token, new_password: password });
  }

  const error =
    reset.error instanceof ApiRequestError
      ? reset.error.detail ?? reset.error.message
      : null;
  const mismatch = confirm.length > 0 && password !== confirm;

  return (
    <main className="auth-page">
      <form className="auth-card" onSubmit={onSubmit} noValidate>
        <header className="auth-card__header">
          <h1>Choose a new password</h1>
          <p>Set a fresh password for your account.</p>
        </header>

        <Input
          id="reset-email"
          label="Email"
          type="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
        />
        <Input
          id="reset-password"
          label="New password"
          type="password"
          autoComplete="new-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
        />
        <Input
          id="reset-confirm"
          label="Confirm new password"
          type="password"
          autoComplete="new-password"
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          required
        />

        {mismatch && (
          <ErrorState title="Passwords don't match" message="Both fields must match." />
        )}
        {!mismatch && error && (
          <ErrorState title="Reset failed" message={error} />
        )}

        <Button
          type="submit"
          loading={reset.isPending}
          disabled={mismatch}
          className="auth-card__submit"
        >
          {reset.isPending ? "Resetting…" : "Set new password"}
        </Button>

        <p className="auth-card__switch">
          <Link to="/login">Back to sign in</Link>
        </p>
      </form>
    </main>
  );
}
