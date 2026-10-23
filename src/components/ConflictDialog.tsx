import { useStore } from "../lib/store";
import type { ConflictField, SyncConflict, Vault } from "../lib/types";
import { HostIcon } from "./common";
import { Button, Modal } from "./ui";

const fieldLabel: Record<ConflictField, string> = {
  label: "Название",
  address: "Адрес",
  port: "Порт",
  user: "Пользователь",
  group: "Группа",
};

export function ConflictDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
  const { ws, update, toast } = useStore();

  const resolve = (c: SyncConflict, keep: "mine" | "theirs") => {
    const left = vault.conflicts.filter((x) => x.id !== c.id);
    update((w) => ({
      ...w,
      hosts: keep === "theirs" ? w.hosts.map((h) => (h.id === c.hostId ? { ...h, ...c.theirs } : h)) : w.hosts,
      vaults: w.vaults.map((v) =>
        v.id === vault.id ? { ...v, conflicts: left, ...(left.length ? {} : { syncStatus: "synced" as const, lastSync: "только что" }) } : v,
      ),
    }));
    if (!left.length) {
      toast({ text: "Конфликт решён, синхронизировано", tone: "success" });
      onClose();
    }
  };

  return (
    <Modal title={vault.conflicts.length > 1 ? `Конфликты · ${vault.name}` : `Конфликт · ${vault.name}`} width={560} onClose={onClose}>
      {vault.conflicts.map((c) => {
        const host = ws.hosts.find((h) => h.id === c.hostId);
        const who = ws.members.find((m) => m.id === c.theirsBy);
        const whoName = who?.name.split(" ")[0] ?? "участника";
        if (!host) return null;
        const fields = (Object.keys(c.theirs) as ConflictField[]).filter((f) => c.theirs[f] !== host[f]);
        return (
          <section className="conflict-item" key={c.id}>
            <div className="conflict-head">
              <HostIcon host={host} size={32} />
              <div>
                <div className="strong">{host.label}</div>
                <div className="muted small">
                  Изменён у вас и у {whoName} · {c.when}
                </div>
              </div>
            </div>

            <div className="diff">
              <span />
              <span className="diff-col">У вас</span>
              <span className="diff-col">У {whoName}</span>
              {fields.map((f) => (
                <div className="diff-row" key={f}>
                  <span className="diff-field">{fieldLabel[f]}</span>
                  <span className="diff-val mono">{String(host[f])}</span>
                  <span className="diff-val mono">{String(c.theirs[f])}</span>
                </div>
              ))}
            </div>

            <div className="conflict-actions">
              <Button onClick={() => resolve(c, "mine")}>Оставить мою</Button>
              <Button variant="primary" onClick={() => resolve(c, "theirs")}>Взять у {whoName}</Button>
            </div>
          </section>
        );
      })}
    </Modal>
  );
}
