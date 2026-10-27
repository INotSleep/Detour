import { useT } from "../i18n";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Check, Eye, EyeOff, Loader2, X, type LucideIcon } from "lucide-react";
import type { ToastInput } from "../lib/store";

/* ---------- buttons ---------- */

type Variant = "primary" | "secondary" | "ghost" | "danger";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: "sm" | "md";
  icon?: LucideIcon;
  loading?: boolean;
}

export function Button({ variant = "secondary", size = "md", icon: Icon, loading, children, className = "", disabled, ...rest }: ButtonProps) {
  return (
    <button className={`btn btn-${variant} btn-${size} ${className}`} disabled={disabled || loading} {...rest}>
      {loading ? <Loader2 size={14} className="spin" /> : Icon && <Icon size={14} />}
      {children}
    </button>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: LucideIcon;
  label: string;
  tipSide?: "top" | "bottom" | "left";
  size?: number;
}

export function IconButton({ icon: Icon, label, tipSide = "bottom", size = 15, className = "", ...rest }: IconButtonProps) {
  return (
    <button className={`icon-btn ${className}`} aria-label={label} data-tip={label} data-tip-side={tipSide} {...rest}>
      <Icon size={size} />
    </button>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="kbd">{children}</kbd>;
}

/* ---------- form ---------- */

export function Field({ label, error, children }: { label: string; error?: string | null; children: ReactNode }) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      {children}
      {error && <span className="field-error">{error}</span>}
    </label>
  );
}

export function PasswordInput({ value, onChange, placeholder, invalid, autoFocus }: { value: string; onChange: (v: string) => void; placeholder?: string; invalid?: boolean; autoFocus?: boolean }) {
  const t = useT().common;
  const [shown, setShown] = useState(false);
  return (
    <div className={`password-input ${invalid ? "invalid" : ""}`}>
      <input
        className="input mono"
        type={shown ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        autoComplete="new-password"
        spellCheck={false}
        autoFocus={autoFocus}
      />
      {value && <IconButton type="button" icon={shown ? EyeOff : Eye} label={shown ? t.hide : t.show} tipSide="left" size={14} onClick={() => setShown((s) => !s)} />}
    </div>
  );
}

export function Checkbox({ checked, onChange, children }: { checked: boolean; onChange: (v: boolean) => void; children: ReactNode }) {
  return (
    <label className="checkbox">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="checkbox-box"><Check size={12} strokeWidth={3} /></span>
      <span>{children}</span>
    </label>
  );
}

export function Segmented<T extends string>({ value, options, onChange }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="segmented" role="radiogroup">
      {options.map((o) => (
        <button type="button" role="radio" aria-checked={value === o.value} key={o.value} className={value === o.value ? "on" : ""} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Switch({ checked, busy, onChange, label }: { checked: boolean; busy?: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} className={`switch ${checked ? "on" : ""} ${busy ? "busy" : ""}`} onClick={() => onChange(!checked)}>
      <span className="knob">{busy && <Loader2 size={10} className="spin" />}</span>
    </button>
  );
}

/* ---------- modal ---------- */

interface ModalProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
  onSubmit?: () => void;
}

export function Modal({ title, onClose, children, footer, width = 440, onSubmit }: ModalProps) {
  const t = useT().common;
  const ref = useRef<HTMLFormElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    ref.current?.querySelector<HTMLElement>("input:not([readonly]), select, textarea, button.btn-primary, button.btn-danger")?.focus();
  }, []);

  return createPortal(
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form
        ref={ref}
        className="modal"
        style={{ width }}
        role="dialog"
        aria-label={title}
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit?.();
        }}
      >
        <div className="modal-head">
          <h2>{title}</h2>
          <IconButton type="button" icon={X} label={t.close} onClick={onClose} />
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </form>
    </div>,
    document.body,
  );
}

export function ConfirmDialog({ title, body, confirm, onConfirm, onClose }: { title: string; body?: ReactNode; confirm: string; onConfirm: () => void; onClose: () => void }) {
  const t = useT().common;
  return (
    <Modal
      title={title}
      onClose={onClose}
      width={400}
      onSubmit={() => {
        onConfirm();
        onClose();
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.cancel}</Button>
          <Button type="submit" variant="danger">{confirm}</Button>
        </>
      }
    >
      {body && <div className="modal-text">{body}</div>}
    </Modal>
  );
}

/* ---------- dropdown menu ---------- */

export interface MenuItem {
  label: string;
  icon?: LucideIcon;
  danger?: boolean;
  hint?: string;
  onSelect: () => void;
}

/** `items` may be a function: then they are made when the menu opens, not on every render (rows of a long list). */
export function Menu({ trigger, items: itemsOrMake, align = "end", header }: { trigger: (open: () => void, isOpen: boolean) => ReactNode; items: MenuItem[] | (() => MenuItem[]); align?: "start" | "end"; header?: string }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !anchor.current || !menu.current) return;
    const a = anchor.current.getBoundingClientRect();
    const m = menu.current.getBoundingClientRect();
    let top = a.bottom + 4;
    if (top + m.height > window.innerHeight - 8) top = a.top - m.height - 4;
    let left = align === "end" ? a.right - m.width : a.left;
    left = Math.max(8, Math.min(left, window.innerWidth - m.width - 8));
    setPos({ top, left });
  }, [open, align]);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!menu.current?.contains(e.target as Node) && !anchor.current?.contains(e.target as Node)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", key);
    };
  }, [open]);

  const items = open ? (typeof itemsOrMake === "function" ? itemsOrMake() : itemsOrMake) : [];

  return (
    <span ref={anchor} className="menu-anchor" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      {trigger(() => { setPos(null); setOpen((o) => !o); }, open)}
      {open &&
        createPortal(
          <div ref={menu} className="menu" role="menu" style={pos ? { top: pos.top, left: pos.left } : { visibility: "hidden", top: 0, left: 0 }}>
            {header && <div className="menu-header">{header}</div>}
            {items.map((it) => (
              <button
                key={it.label}
                role="menuitem"
                className={it.danger ? "danger" : ""}
                onClick={() => {
                  setOpen(false);
                  it.onSelect();
                }}
              >
                {it.icon && <it.icon size={14} />}
                <span>{it.label}</span>
                {it.hint && <span className="menu-hint">{it.hint}</span>}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </span>
  );
}

/** A menu at the pointer, for a right click. */
export function ContextMenu({ at, items, onClose }: { at: { x: number; y: number }; items: MenuItem[]; onClose: () => void }) {
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!menu.current) return;
    const m = menu.current.getBoundingClientRect();
    const top = at.y + m.height > window.innerHeight - 8 ? Math.max(8, at.y - m.height) : at.y;
    const left = Math.max(8, Math.min(at.x, window.innerWidth - m.width - 8));
    setPos({ top, left });
  }, [at]);

  useEffect(() => {
    const away = (e: MouseEvent) => !menu.current?.contains(e.target as Node) && onClose();
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", away, true);
    window.addEventListener("keydown", key);
    window.addEventListener("wheel", onClose);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("keydown", key);
      window.removeEventListener("wheel", onClose);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return createPortal(
    <div ref={menu} className="menu" role="menu" style={pos ? { top: pos.top, left: pos.left } : { visibility: "hidden", top: 0, left: 0 }} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it) => (
        <button
          key={it.label}
          role="menuitem"
          className={it.danger ? "danger" : ""}
          onClick={() => {
            onClose();
            it.onSelect();
          }}
        >
          {it.icon && <it.icon size={14} />}
          <span>{it.label}</span>
          {it.hint && <span className="menu-hint">{it.hint}</span>}
        </button>
      ))}
    </div>,
    document.body,
  );
}

/* ---------- toasts ---------- */

interface ToastState extends ToastInput {
  id: number;
  leaving?: boolean;
}

let toastSeq = 0;

export function useToaster() {
  const [toasts, setToasts] = useState<ToastState[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((ts) => ts.map((t) => (t.id === id ? { ...t, leaving: true } : t)));
    setTimeout(() => setToasts((ts) => ts.filter((t) => t.id !== id)), 160);
  }, []);

  const push = useCallback(
    (t: ToastInput) => {
      const id = ++toastSeq;
      setToasts((ts) => [...ts.slice(-3), { ...t, id }]);
      setTimeout(() => dismiss(id), t.duration ?? (t.action ? 6000 : 3200));
    },
    [dismiss],
  );

  const view = createPortal(
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.tone ?? "default"} ${t.leaving ? "leaving" : ""}`}>
          {t.tone === "success" && <Check size={15} className="toast-icon" />}
          {t.tone === "error" && <X size={15} className="toast-icon" />}
          <span className="toast-text">{t.text}</span>
          {t.action && (
            <button
              className="toast-action"
              onClick={() => {
                t.action!.run();
                dismiss(t.id);
              }}
            >
              {t.action.label}
            </button>
          )}
        </div>
      ))}
    </div>,
    document.body,
  );

  return { push, view };
}

/* ---------- misc ---------- */

export function Empty({ icon: Icon, title, action }: { icon: LucideIcon; title: string; action?: ReactNode }) {
  return (
    <div className="empty">
      <div className="empty-icon"><Icon size={22} /></div>
      <div className="empty-title">{title}</div>
      {action}
    </div>
  );
}

export function PageHeader({ title, count, children }: { title: string; count?: number; children?: ReactNode }) {
  return (
    <header className="page-header">
      <h1>
        {title}
        {count !== undefined && <span className="count">{count}</span>}
      </h1>
      <div className="page-actions">{children}</div>
    </header>
  );
}
