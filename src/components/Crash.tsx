// When the window breaks: its error goes into the log, and instead of an empty window there is
// what broke and a way back.

import { Component, type ErrorInfo, type ReactNode } from "react";
import { TriangleAlert } from "lucide-react";
import { api } from "../lib/api";
import { copyText } from "../lib/store";
import { tr } from "../i18n";
import { Button } from "./ui";

/** An error as the log keeps it: its message and where it came from. */
export function errorReport(e: unknown, more = ""): string {
  const err = e instanceof Error ? e : new Error(typeof e === "string" ? e : JSON.stringify(e));
  return [`${err.name}: ${err.message}`, err.stack?.split("\n").slice(1).join("\n") ?? "", more].filter(Boolean).join("\n");
}

let sent = 0;
let since = Date.now();
/** Writes an error of the window into the log; a storm of them is cut short. */
export function logError(report: string) {
  if (Date.now() - since > 60_000) [sent, since] = [0, Date.now()];
  if (++sent > 20) return;
  void api.logWrite("error", report).catch(() => {});
}

/** Errors that happen outside rendering (handlers, promises) go into the log too. */
export function catchWindowErrors() {
  window.addEventListener("error", (e) => logError(errorReport(e.error ?? e.message)));
  window.addEventListener("unhandledrejection", (e) => logError(errorReport(e.reason, "(unhandled promise)")));
}

/** Opens the window again on its start: the view it showed (a vault, Mine) may be what broke. */
function reload() {
  try {
    for (const k of Object.keys(localStorage)) if (k === "detour.vault" || k.startsWith("detour.groups.")) localStorage.removeItem(k);
  } catch {
    /* no storage: just reload */
  }
  location.reload();
}

export class Crash extends Component<{ children: ReactNode }, { report: string | null; copied: boolean }> {
  state = { report: null as string | null, copied: false };

  static getDerivedStateFromError(e: unknown) {
    return { report: errorReport(e), copied: false };
  }

  componentDidCatch(e: unknown, info: ErrorInfo) {
    const report = errorReport(e, info.componentStack ? `in${info.componentStack}` : "");
    this.setState({ report });
    logError(report);
  }

  render() {
    const report = this.state.report;
    if (report === null) return this.props.children;
    const t = tr().app.crash;
    const copy = async () => {
      const log = await api.logTail(32 * 1024).catch(() => null);
      const ok = await copyText(log ? `${report}\n\n--- ${log.path}\n${log.text}` : report);
      this.setState({ copied: ok });
    };
    return (
      <div className="crash">
        <div className="crash-box">
          <TriangleAlert size={28} className="err-text" />
          <h1>{t.title}</h1>
          <pre className="crash-error mono">{report.split("\n").slice(0, 8).join("\n")}</pre>
          <div className="crash-actions">
            <Button variant="primary" onClick={reload} autoFocus>{t.reload}</Button>
            <Button onClick={() => void copy()}>{this.state.copied ? t.copied : t.copy}</Button>
            {api.real && <Button variant="ghost" onClick={() => void api.logTail(0).then((l) => api.openLocal(l.dir)).catch(() => {})}>{t.openLog}</Button>}
          </div>
        </div>
      </div>
    );
  }
}
