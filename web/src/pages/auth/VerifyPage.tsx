import { useState, type FormEvent } from "react";
import { Link, useLocation } from "react-router-dom";
import { useVerifyEmail } from "../../api/hooks";
import { ApiRequestError } from "../../api/client";
import { Button } from "../../components/Button/Button";
import { Input } from "../../components/Input/Input";
import { ErrorState } from "../../components/Status/Status";
import "./auth.css";

export function VerifyPage() {
  const location = useLocation();
  const registeredEmail = (location.state as { email?: string } | null)?.email;
  const [token, setToken] = useState("");
  const [verified, setVerified] = useState(false);
  const verify = useVerifyEmail();

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    verify.mutate(
      { token: token.trim() },
      {
        onSuccess: () => {
          setVerified(true);
        },
      },
    );
  }

  const error =
    verify.error instanceof ApiRequestError
      ? verify.error.detail ?? verify.error.message
      : null;

  if (verified) {
    return (
      <main className="auth-page">
        <div className="auth-card" role="status">
          <header className="auth-card__header">
            <h1>Email verified</h1>
            <p>
              Your account is confirmed{registeredEmail ? ` for ${registeredEmail}` : ""}. You
              can sign in now.
            </p>
          </header>
          <Link to="/login">
            <Button className="auth-card__submit">Go to sign in</Button>
          </Link>
        </div>
      </main>
    );
  }

  return (
    <main className="auth-page">
      <form className="auth-card" onSubmit={onSubmit} noValidate>
        <header className="auth-card__header">
          <h1>Check your email</h1>
          <p>
            {registeredEmail
              ? `We sent a verification code to ${registeredEmail}.`
              : "Enter the verification code you received by email."}
          </p>
        </header>

        <Input
          id="verify-token"
          label="Verification code"
          autoComplete="one-time-code"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          required
        />

        {error && <ErrorState title="Verification failed" message={error} />}

        <Button type="submit" loading={verify.isPending} className="auth-card__submit">
          {verify.isPending ? "Verifying…" : "Verify email"}
        </Button>

        <p className="auth-card__hint">
          In this development build the code is returned by the register endpoint — copy it
          from there.
        </p>
        <p className="auth-card__switch">
          <Link to="/login">Back to sign in</Link>
        </p>
      </form>
    </main>
  );
}
