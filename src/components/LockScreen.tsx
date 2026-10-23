import { useState } from "react";
import { Lock } from "lucide-react";
import { api, errorText, isCoreError, type VaultView } from "../lib/api";
import { Button, Field, PasswordInput } from "./ui";

const MIN_LENGTH = 8;

export function LockScreen({ mode, onUnlocked }: { mode: "setup" | "unlock"; onUnlocked: (view: VaultView | null) => void }) {
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);

  const setup = mode === "setup";
  const tooShort = setup && password.length < MIN_LENGTH;
  const mismatch = setup && password !== repeat;

  const submit = async () => {
    if (busy || !password) return;
    if (tooShort) return setError(`Минимум ${MIN_LENGTH} символов`);
    if (mismatch) return setError("Пароли не совпадают");
    setBusy(true);
    setError(null);
    try {
      onUnlocked(setup ? await api.vaultCreate(password) : await api.vaultUnlock(password));
    } catch (e) {
      setError(isCoreError(e) && e.kind === "wrongMasterPassword" ? "Неверный мастер-пароль" : errorText(e));
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
        <h1>{setup ? "Мастер-пароль" : "Detour заблокирован"}</h1>

        <Field label={setup ? "Новый мастер-пароль" : "Мастер-пароль"}>
          <PasswordInput value={password} onChange={(v) => { setPassword(v); setError(null); }} invalid={!!error} autoFocus />
        </Field>
        {setup && (
          <Field label="Ещё раз">
            <PasswordInput value={repeat} onChange={(v) => { setRepeat(v); setError(null); }} />
          </Field>
        )}
        {error && <span className="field-error">{error}</span>}
        {setup && <span className="muted small">Забытый мастер-пароль восстановить нельзя.</span>}

        <Button type="submit" variant="primary" className="block" loading={busy} disabled={!password}>
          {setup ? "Создать vault" : "Разблокировать"}
        </Button>
      </form>
    </div>
  );
}
