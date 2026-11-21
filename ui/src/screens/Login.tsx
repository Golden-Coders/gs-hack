import { useState, type FormEvent } from "react";
import Button from "@atlaskit/button/new";
import Heading from "@atlaskit/heading";
import { Text } from "@atlaskit/primitives/compiled";
import { loginWithPassword } from "../api/session";
import { Card, ErrorBox, Field, TextInput } from "../ui/primitives";
import "./Login.css";

export default function Login() {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!username.trim() || !password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await loginWithPassword(username, password);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign in failed.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="login-page">
      <div className="login-wrap">
        <Card>
          <form className="login-form" onSubmit={(e) => void handleSubmit(e)} noValidate>
            <div className="login-head">
              <Heading size="large" as="h1">
                AlertHub
              </Heading>
              <Text as="p" color="color.text.subtle">
                Sign in to your operator account
              </Text>
            </div>

            {error && <ErrorBox message={error} />}

            <Field label="Username">
              {(f) => (
                <TextInput
                  {...f}
                  value={username}
                  onChange={(e) => setUsername(e.currentTarget.value)}
                  placeholder="e.g. anna, jan, krzysztof..."
                  autoComplete="username"
                  autoFocus
                />
              )}
            </Field>

            <Field label="Password">
              {(f) => (
                <TextInput
                  {...f}
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.currentTarget.value)}
                  placeholder="••••••••"
                  autoComplete="current-password"
                />
              )}
            </Field>

            <Button
              type="submit"
              appearance="primary"
              isLoading={busy}
              isDisabled={busy || !username.trim() || !password}
            >
              Sign in
            </Button>
          </form>
        </Card>
      </div>
    </main>
  );
}
