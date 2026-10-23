import { useState } from "react";
import { Lock } from "lucide-react";
import { useT } from "../i18n";
import { api, errorText, type VaultView } from "../lib/api";
import { Button, Field, PasswordInput } from "./ui";

const MIN_LENGTH = 8;

export function LockScreen({ mode, onUnlocked }: { mode: "setup" | "unlock"; onUnlocked: (view: VaultView | null) => void }) {
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const t = useT().app.lock;

  const setup = mode === "setup";
  const tooShort = setup && password.length < MIN_LENGTH;
  const mismatch = setup && password !== repeat;

  const submit = async () => {
    if (busy || !password) return;
    if (tooShort) return setError(t.minLength(MIN_LENGTH));
    if (mismatch) return setError(t.mismatch);
    setBusy(true);
    setError(null);
    try {
      onUnlocked(setup ? await api.vaultCreate(password) : await api.vaultUnlock(password));
    } catch (e) {
      setError(errorText(e));
      setShake((n) => n + 1);
      setPassword("");
      setBusy(false);
    }
  };

  return (
    <div className="lock-screen">
      <form
        key={shake}
        className={`lock-card ${shake ? "shake" : ""}`}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className="lock-icon">{setup ? <img src="/logo.svg" alt="" /> : <Lock size={22} />}</div>
        <h1>{setup ? t.setupTitle : t.lockedTitle}</h1>

        <Field label={setup ? t.newPassword : t.password}>
          <PasswordInput value={password} onChange={(v) => { setPassword(v); setError(null); }} invalid={!!error} autoFocus />
        </Field>
        {setup && (
          <Field label={t.repeat}>
            <PasswordInput value={repeat} onChange={(v) => { setRepeat(v); setError(null); }} />
          </Field>
        )}
        {error && <span className="field-error">{error}</span>}
        {setup && <span className="muted small">{t.noRecovery}</span>}

        <Button type="submit" variant="primary" className="block" loading={busy} disabled={!password}>
          {setup ? t.create : t.unlock}
        </Button>
      </form>
    </div>
  );
}
