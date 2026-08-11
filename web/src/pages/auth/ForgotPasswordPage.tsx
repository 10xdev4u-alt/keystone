import { useState, type FormEvent } from "react";
import { Link } from "react-router-dom";
import { useForgotPassword } from "../../api/hooks";
import { ApiRequestError } from "../../api/client";
import { Button } from "../../components/Button/Button";
import { Input } from "../../components/Input/Input";
import { ErrorState } from "../../components/Status/Status";
import "./auth.css";

export function ForgotPasswordPage() {
  const [email, setEmail] = useState("");
  const forgot = useForgotPassword();
  const [sent, setSent] = useState(false);

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    forgot.mutate(
      { email },
      {
        onSuccess: () => setSent(true),
      },
    );
  }

  const error =
    forgot.error instanceof ApiRequestError
      ? forgot.error.detail ?? forgot.error.message
      : null;

  return (
    <main className="auth-page">
      <form className="auth-card" onSubmit={onSubmit} noValidate>
        <header className="auth-card__header">
          <h1>Reset your password</h1>
          <p>Enter your account email and we'll send reset instructions.</p>
        </header>

        {sent ? (
          <div className="auth-card__success" role="status">
            <p>If that address is registered, a reset token is on the way.</p>
            <p className="auth-card__hint">
              In this development build the token is returned directly — check
              the network response or your terminal.
            </p>
            <Link to="/login" className="auth-card__link">
              Back to sign in
            </Link>
          </div>
        ) : (
          <>
            <Input
              id="forgot-email"
              label="Email"
              type="email"
              autoComplete="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
            />

            {error && <ErrorState title="Request failed" message={error} />}

            <Button
              type="submit"
              loading={forgot.isPending}
              className="auth-card__submit"
            >
              {forgot.isPending ? "Sending…" : "Send reset link"}
            </Button>

            <p className="auth-card__switch">
              Remembered it? <Link to="/login">Sign in</Link>
            </p>
          </>
        )}
      </form>
    </main>
  );
}
