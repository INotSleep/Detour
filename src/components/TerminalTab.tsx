import { Fragment, useEffect, useRef, useState } from "react";
import type { Host } from "../lib/types";
import type { SessionStatus } from "../lib/fakeShell";
import { uid } from "../lib/store";
import { TerminalView, type SplitDir } from "./TerminalView";

interface Pane {
  id: string;
  command?: string;
}

interface Props {
  host: Host;
  active: boolean;
  initialCommand?: string;
  onStatus: (s: SessionStatus) => void;
  /** The last pane closed. */
  onExit: () => void;
}

/** Most panes a tab splits into. */
const MAX_PANES = 6;

/** A terminal tab: one session, or several to the same host side by side or stacked. */
export function TerminalTab({ host, active, initialCommand, onStatus, onExit }: Props) {
  const [panes, setPanes] = useState<Pane[]>(() => [{ id: uid("p"), command: initialCommand }]);
  const [dir, setDir] = useState<SplitDir>("row");
  // Share of the tab per pane, summing to 1.
  const [sizes, setSizes] = useState<number[]>([1]);
  const [focused, setFocused] = useState(panes[0]!.id);
  const statuses = useRef(new Map<string, SessionStatus>());
  const box = useRef<HTMLDivElement>(null);

  const report = () => onStatus(statuses.current.get(focused) ?? "connecting");
  useEffect(report, [focused]); // eslint-disable-line react-hooks/exhaustive-deps

  const split = (after: string, d: SplitDir) => {
    if (panes.length >= MAX_PANES) return;
    const pane = { id: uid("p") };
    const i = panes.findIndex((p) => p.id === after);
    const next = [...panes.slice(0, i + 1), pane, ...panes.slice(i + 1)];
    if (panes.length === 1) setDir(d);
    setPanes(next);
    setSizes(next.map(() => 1 / next.length));
    setFocused(pane.id);
  };

  const close = (id: string) => {
    statuses.current.delete(id);
    const next = panes.filter((p) => p.id !== id);
    if (next.length === 0) return onExit();
    setPanes(next);
    setSizes(next.map(() => 1 / next.length));
    if (focused === id) setFocused(next[Math.max(0, panes.findIndex((p) => p.id === id) - 1)]!.id);
  };

  /** Drags the divider after pane `i`. */
  const drag = (i: number, e: React.MouseEvent) => {
    e.preventDefault();
    const rect = box.current!.getBoundingClientRect();
    const total = dir === "row" ? rect.width : rect.height;
    const start = dir === "row" ? e.clientX : e.clientY;
    const [a, b] = [sizes[i]!, sizes[i + 1]!];
    const min = 120 / total;
    const move = (ev: MouseEvent) => {
      const delta = ((dir === "row" ? ev.clientX : ev.clientY) - start) / total;
      const na = Math.min(Math.max(a + delta, min), a + b - min);
      setSizes((s) => s.map((v, j) => (j === i ? na : j === i + 1 ? a + b - na : v)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("resizing", `resizing-${dir}`);
    };
    document.body.classList.add("resizing", `resizing-${dir}`);
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div className={`term-panes ${dir}`} ref={box}>
      {panes.map((p, i) => (
        <Fragment key={p.id}>
          {i > 0 && <div className={`term-divider ${dir}`} onMouseDown={(e) => drag(i - 1, e)} />}
          <div className={`term-pane ${panes.length > 1 && focused === p.id ? "focused" : ""}`} style={{ flexGrow: sizes[i] ?? 1 }}>
            <TerminalView
              host={host}
              active={active && focused === p.id}
              initialCommand={p.command}
              onStatus={(s) => {
                statuses.current.set(p.id, s);
                if (p.id === focused) report();
              }}
              onExit={() => close(p.id)}
              onSplit={panes.length < MAX_PANES ? (d) => split(p.id, d) : undefined}
              splitDirs={panes.length > 1 ? [dir] : ["row", "col"]}
              onFocus={() => setFocused(p.id)}
            />
          </div>
        </Fragment>
      ))}
    </div>
  );
}
