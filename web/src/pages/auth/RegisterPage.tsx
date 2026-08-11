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

        <p className="auth-card__switch">
          Already have an account? <Link to="/login">Sign in</Link>
        </p>
      </form>
    </main>
  );
}
