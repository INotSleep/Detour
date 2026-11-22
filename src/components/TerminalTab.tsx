import { useEffect, useReducer, useRef } from "react";
import type { SessionStatus } from "../lib/fakeShell";
import { useStore } from "../lib/store";
import { MAX_PANES, panes as panesOf, place, resize, sharesAt, type Layout, type Side, type SplitDir } from "../lib/panes";
import { TerminalView } from "./TerminalView";
import { AgentRuns } from "./Agents";

interface Props {
  layout: Layout;
  focused: string;
  active: boolean;
  /** The AI agent whose session it is: what it runs shows under the terminal. */
  agent?: string;
  agentSession?: string;
  onLayout: (fn: (l: Layout) => Layout) => void;
  onFocus: (paneId: string) => void;
  /** The status of the pane with focus. */
  onStatus: (s: SessionStatus) => void;
  onSplit: (paneId: string, dir: SplitDir, otherHost: boolean) => void;
  onClosePane: (paneId: string) => void;
  onMoveFocus: (paneId: string, side: Side) => void;
  /** Takes the pane out into a tab of its own. */
  onDetach: (paneId: string) => void;
}

/** A terminal tab: one session, or several split any way, each pane to its own host. */
export function TerminalTab({ layout, focused, active, agent, agentSession, onLayout, onFocus, onStatus, onSplit, onClosePane, onMoveFocus, onDetach }: Props) {
  const { ws } = useStore();
  const statuses = useRef(new Map<string, SessionStatus>());
  // Pane labels show each pane's status: drawn again when one changes.
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  const box = useRef<HTMLDivElement>(null);
  const { panes, dividers } = place(layout);
  const many = panes.length > 1;
  // With panes on different hosts each says where it is connected.
  const labelled = new Set(panes.map((p) => p.pane.hostId)).size > 1;

  const report = () => onStatus(statuses.current.get(focused) ?? "connecting");
  useEffect(report, [focused]); // eslint-disable-line react-hooks/exhaustive-deps

  // A pane whose host was deleted goes.
  const gone = panes.filter((p) => !ws.hosts.some((h) => h.id === p.pane.hostId)).map((p) => p.pane.id);
  useEffect(() => {
    for (const id of gone) onClosePane(id);
  }, [gone.join()]); // eslint-disable-line react-hooks/exhaustive-deps

  const drag = (d: (typeof dividers)[number], e: React.MouseEvent) => {
    e.preventDefault();
    const rect = box.current!.getBoundingClientRect();
    const total = d.dir === "row" ? rect.width * d.node.w : rect.height * d.node.h;
    const start = d.dir === "row" ? e.clientX : e.clientY;
    const shares = sharesAt(layout, d.path);
    const [a, b] = [shares[d.index]!, shares[d.index + 1]!];
    const min = Math.min(120 / total, (a + b) / 2);
    const move = (ev: MouseEvent) => {
      const delta = ((d.dir === "row" ? ev.clientX : ev.clientY) - start) / total;
      onLayout((l) => resize(l, d.path, d.index, Math.min(Math.max(a + delta, min), a + b - min)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("resizing", `resizing-${d.dir}`);
    };
    document.body.classList.add("resizing", `resizing-${d.dir}`);
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const pct = (v: number) => `${v * 100}%`;
  return (
    <>
      <div className="term-panes" ref={box} data-tab-panes>
        {panes.map(({ pane: p, rect }) => {
          const host = ws.hosts.find((h) => h.id === p.hostId);
          if (!host) return null;
          return (
            <div
              key={p.id}
              className={`term-pane ${many && focused === p.id ? "focused" : ""} ${labelled ? "labelled" : ""}`}
              data-pane={p.id}
              style={{ left: pct(rect.x), top: pct(rect.y), width: pct(rect.w), height: pct(rect.h) }}
            >
              {labelled && (
                <div className="term-pane-host">
                  <span className={`session-dot ${statuses.current.get(p.id) ?? "connecting"}`} />
                  <span className="ellipsis">{host.label}</span>
                </div>
              )}
              <div className="term-pane-body">
                <TerminalView
                  host={host}
                  paneId={p.id}
                  active={active && focused === p.id}
                  initialCommand={p.command}
                  attach={p.attach}
                  onStatus={(s) => {
                    statuses.current.set(p.id, s);
                    if (p.id === focused) report();
                    if (labelled) redraw();
                  }}
                  onExit={() => onClosePane(p.id)}
                  onSplit={panes.length < MAX_PANES ? (d, other) => onSplit(p.id, d, other) : undefined}
                  onMoveFocus={(side) => onMoveFocus(p.id, side)}
                  onDetach={many ? () => onDetach(p.id) : undefined}
                  onFocus={() => focused !== p.id && onFocus(p.id)}
                />
              </div>
            </div>
          );
        })}
        {dividers.map((d) => (
          <div
            key={`${d.path.join(".")}:${d.index}`}
            className={`term-divider ${d.dir}`}
            style={{ left: pct(d.x), top: pct(d.y), ...(d.dir === "row" ? { height: pct(d.h) } : { width: pct(d.w) }) }}
            onMouseDown={(e) => drag(d, e)}
          />
        ))}
      </div>
      {agent && agentSession && <AgentRuns sessionId={agentSession} agent={agent} />}
    </>
  );
}

export const paneCount = (l: Layout) => panesOf(l).length;
