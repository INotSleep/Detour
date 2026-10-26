// Colours for terminal output that has none of its own: the prompt, addresses,
// paths, sizes, percentages, dates and status words. Colours sent by the server
// always win: only text in the default colour (no dim, inverse or background)
// is painted, and full screen programs (the alternate screen) are left alone.
// Colours are the theme's ANSI ones, so highlights suit any theme.

const SGR = { green: "32", yellow: "33", red: "31", blue: "34", magenta: "35", cyan: "36", dim: "90" } as const;
type Colour = keyof typeof SGR;

const paint = (c: Colour, s: string) => `\x1b[${SGR[c]}m${s}\x1b[39m`;

/** "word" also as "Word" and "WORD". */
const words = (list: string) => list.split(" ").flatMap((w) => [w, w[0]!.toUpperCase() + w.slice(1), w.toUpperCase()]).join("|");

const OK = words("active running ok success successful successfully succeeded completed enabled up online healthy passed done ready listen listening estab established connected started accepted");
const WARN = words("warning warn warnings deprecated degraded pending retrying skipped disabled exited activating deactivating reloading restarting");
const ERR = words("error errors err failed failure fail fatal denied refused critical crit emerg panic inactive dead down unreachable invalid killed exception rejected unhealthy timeout aborted");

const MONTH = "Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec";
const DAY = "Mon|Tue|Wed|Thu|Fri|Sat|Sun";
const HEX = "[0-9a-fA-F]{1,4}";
// Letters and digits of any script, for word edges.
const W = "\\p{L}\\p{N}_";

// One pass over the text: at each place the first alternative that matches wins.
const RULES = new RegExp(
  [
    // user@host:path$ and [user@host dir]$, as bash prints them without colours.
    `(?<![${W}.-])(?<pu>[a-z_][\\w.-]*@[\\w.-]+):(?<pp>[~/][^\\s$#]*)(?<ps>[$#])(?= |$)`,
    `\\[(?<bu>[a-z_][\\w.-]*@[\\w.-]+) (?<bp>[^\\]\\s]+)\\](?<bs>[$#])(?= |$)`,
    `(?<url>\\b(?:https?|ftp|ssh|wss?)://[^\\s'"<>()\\x00-\\x1f]*[^\\s'"<>().,;:!?\\x00-\\x1f])`,
    // nginx: 26/Oct/2026:13:51:33 +0000
    `(?<date>(?<![${W}])\\d{2}/(?:${MONTH})/\\d{4}:\\d{2}:\\d{2}:\\d{2}(?: [+-]\\d{4})?` +
      // 2026-10-26 13:51:33, ISO with a zone, "Mon 2026-10-26 ... UTC"
      `|(?<![${W}])(?:(?:${DAY}) )?\\d{4}[-/]\\d{2}[-/]\\d{2}(?:[T ]\\d{2}:\\d{2}(?::\\d{2}(?:[.,]\\d+)?)?(?:Z|[+-]\\d{2}:?\\d{2}| UTC| GMT)?)?(?![${W}])` +
      // Oct 26 13:51:33, Mon Oct 26 13:51:33 UTC 2026, Oct 26  2025
      `|(?<![${W}])(?:(?:${DAY}),? )?(?:${MONTH}) {1,2}\\d{1,2}(?: \\d{2}:\\d{2}(?::\\d{2})?| {1,2}\\d{4})(?: (?:UTC|GMT))?(?: \\d{4})?(?![${W}])` +
      // 13:51:33
      `|(?<![${W}:.])\\d{1,2}:\\d{2}:\\d{2}(?:[.,]\\d+)?(?![${W}:]))`,
    // [::]:22, *:80, localhost:8080, example.com:443, 10.0.0.1:22
    `(?<addr>(?<![${W}.-])(?:\\[[0-9a-fA-F:.]+\\]|\\*|localhost|(?:[a-zA-Z0-9-]+\\.)+[a-zA-Z]{2,}|(?:\\d{1,3}\\.){3}\\d{1,3}):\\d{1,5}(?![${W}:]|\\.\\d)` +
      // MAC
      `|(?<![${W}:])(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}(?![${W}:])` +
      // IPv6, with :: or all eight groups
      `|(?<![${W}:])(?:(?:${HEX}:){7}${HEX}|(?:${HEX}:){1,7}:(?:${HEX}(?::${HEX}){0,6})?|::${HEX}(?::${HEX}){0,6})(?:/\\d{1,3})?(?![${W}:])` +
      // IPv4, with a prefix length
      `|(?<![${W}.])(?:\\d{1,3}\\.){3}\\d{1,3}(?:/\\d{1,2})?(?![${W}]|\\.\\d))`,
    // /var/log/x, ~/x, ./x, ../x, C:\x
    `(?<path>(?<![${W}.~/\\\\-])(?:~|\\.{1,2})?(?:/[${W}.@%+,~-]+)+/?(?<![.,])` +`|(?<![${W}])[A-Za-z]:\\\\(?:[^\\\\\\s:*?"<>|]+\\\\?)*)`,
    // HTTP/1.1 200 OK; an access log's "GET / HTTP/1.1" 200
    `(?<=HTTP/\\d(?:\\.\\d)? |" )(?<http>[1-5]\\d{2})(?![${W}])`,
    `(?<pct>(?<![${W}.])\\d+(?:[.,]\\d+)?%)`,
    // 412.6M, 80G, 1.5GiB, 48ms, 12 MB, 23 days
    `(?<num>(?<![${W}.])\\d+(?:[.,]\\d+)?(?:(?:[KMGTPE]i?B?|kB|B|bytes|ms|us|µs|ns|s|sec|min|h|d)|(?: (?:[KMGTPE]i?B|kB|bytes|ms|sec|secs|seconds?|mins?|minutes?|hours?|days?|weeks?)))(?![${W}]))`,
    // Not "Active:", a label.
    `(?<ok>(?<![${W}-])(?:${OK})(?![${W}-]|\\.\\w|:))`,
    `(?<warn>(?<![${W}-])(?:${WARN})(?![${W}-]|\\.\\w))`,
    `(?<err>(?<![${W}-])(?:${ERR})(?![${W}-]|\\.\\w))`,
  ].join("|"),
  "gu",
);

const httpColour = (code: string): Colour => (code[0] === "2" ? "green" : code[0] === "3" ? "cyan" : code[0] === "4" ? "yellow" : code[0] === "5" ? "red" : "dim");
const pctColour = (v: number): Colour => (v >= 90 ? "red" : v >= 70 ? "yellow" : "green");

/** `text` with highlights; it must be plain text in the default colour, without escape sequences. */
export function highlightText(text: string): string {
  return text.replace(RULES, (m, ...rest) => {
    const g = rest[rest.length - 1] as Record<string, string | undefined>;
    if (g.pu !== undefined) return `${paint("green", g.pu)}:${paint("blue", g.pp!)}${g.ps}`;
    if (g.bu !== undefined) return `[${paint("green", g.bu)} ${paint("blue", g.bp!)}]${g.bs}`;
    if (g.url !== undefined) return paint("cyan", m);
    if (g.date !== undefined) return paint("dim", m);
    if (g.addr !== undefined) return paint("cyan", m);
    if (g.path !== undefined) return paint("blue", m);
    if (g.http !== undefined) return paint(httpColour(m), m);
    if (g.pct !== undefined) return paint(pctColour(parseFloat(m.replace(",", "."))), m);
    if (g.num !== undefined) return paint("magenta", m);
    if (g.ok !== undefined) return paint("green", m);
    if (g.warn !== undefined) return paint("yellow", m);
    if (g.err !== undefined) return paint("red", m);
    return m;
  });
}

/* ---------- the stream ---------- */

const ESC = 0x1b;
/** Below this a write is typed echo or a prompt and goes out at once; above it a word cut at the end waits for the rest. */
const HOLD_FROM = 256;
const HOLD_MAX = 200;

/**
 * Reads the terminal's output as it comes, keeping track of escape sequences,
 * the current colours and the alternate screen, and adds highlights to plain text.
 */
export class OutputHighlighter {
  private state: "ground" | "esc" | "escInter" | "csi" | "osc" | "str" | "strEsc" = "ground";
  private csi = "";
  private fg = false;
  private bg = false;
  private faint = false;
  private inverse = false;
  private hidden = false;
  private alt = false;
  private held = "";

  /** A word cut at the end of the last write, waiting for the rest. */
  get waiting() {
    return this.held;
  }

  /** `text` with highlights when `on`; either way the state follows the stream. */
  write(text: string, on = true): string {
    if (this.held) {
      text = this.held + text;
      this.held = "";
    }
    let out = "";
    // Where what hasn't gone out yet starts: plain text in "ground", a sequence otherwise.
    let from = 0;
    const text_ = (end: number) => {
      if (end > from) out += on && this.plain() ? highlightText(text.slice(from, end)) : text.slice(from, end);
      from = end;
    };
    // Sequences pass through untouched.
    const seq = (last: number) => {
      out += text.slice(from, last + 1);
      from = last + 1;
      this.state = "ground";
    };
    for (let i = 0; i < text.length; i++) {
      const c = text.charCodeAt(i);
      switch (this.state) {
        case "ground":
          if (c === ESC) {
            text_(i);
            this.state = "esc";
          }
          break;
        case "esc":
          if (c === 0x5b) {
            this.state = "csi";
            this.csi = "";
          } else if (c === 0x5d) this.state = "osc";
          else if (c === 0x50 || c === 0x58 || c === 0x5e || c === 0x5f) this.state = "str";
          else if (c >= 0x20 && c <= 0x2f) this.state = "escInter";
          else {
            if (c === 0x63) this.reset();
            seq(i);
          }
          break;
        case "escInter":
          if (c >= 0x30 && c <= 0x7e) seq(i);
          break;
        case "csi":
          if (c >= 0x40 && c <= 0x7e) {
            this.onCsi(this.csi, text[i]!);
            seq(i);
          } else if (c === 0x18 || c === 0x1a) seq(i);
          else this.csi += text[i];
          break;
        case "osc":
        case "str":
          if (c === 0x07 && this.state === "osc") seq(i);
          else if (c === ESC) this.state = "strEsc";
          break;
        case "strEsc":
          // ESC \ ends the string; anything else is cut short by a new sequence.
          if (c === 0x5c) seq(i);
          else {
            out += text.slice(from, i - 1);
            from = i - 1;
            this.state = "esc";
            i--;
          }
          break;
      }
    }
    if (this.state !== "ground") {
      // A sequence cut in two: what came of it so far goes out as it is.
      out += text.slice(from);
      return out;
    }
    let end = text.length;
    // A long write cut in the middle of a word: the word waits for the next write, or for flush().
    if (on && this.plain() && text.length >= HOLD_FROM) {
      const word = text.slice(Math.max(from, end - HOLD_MAX)).match(/[^\s\x00-\x1f]+$/u);
      if (word && word.index! > 0) {
        end -= word[0].length;
        this.held = text.slice(end);
      }
    }
    text_(end);
    return out;
  }

  /** What waits, now. */
  flush(on = true): string {
    const rest = this.held;
    this.held = "";
    return rest && on && this.plain() ? highlightText(rest) : rest;
  }

  private plain() {
    return !this.alt && !this.fg && !this.bg && !this.faint && !this.inverse && !this.hidden;
  }

  private reset() {
    this.fg = this.bg = this.faint = this.inverse = this.hidden = this.alt = false;
  }

  private onCsi(params: string, final: string) {
    if (final === "m") return this.onSgr(params);
    if ((final === "h" || final === "l") && params.startsWith("?")) {
      for (const p of params.slice(1).split(";")) if (p === "47" || p === "1047" || p === "1049") this.alt = final === "h";
    }
  }

  private onSgr(params: string) {
    const ps = params === "" ? ["0"] : params.split(";");
    for (let i = 0; i < ps.length; i++) {
      const [head, sub] = [ps[i]!.split(":")[0]!, ps[i]!.includes(":")];
      const n = head === "" ? 0 : Number(head);
      if (n === 0) this.fg = this.bg = this.faint = this.inverse = this.hidden = false;
      else if (n === 2) this.faint = true;
      else if (n === 22) this.faint = false;
      else if (n === 7) this.inverse = true;
      else if (n === 27) this.inverse = false;
      else if (n === 8) this.hidden = true;
      else if (n === 28) this.hidden = false;
      else if ((n >= 30 && n <= 37) || (n >= 90 && n <= 97)) this.fg = true;
      else if (n === 39) this.fg = false;
      else if ((n >= 40 && n <= 47) || (n >= 100 && n <= 107)) this.bg = true;
      else if (n === 49) this.bg = false;
      if (n === 38 || n === 48 || n === 58) {
        if (n === 38) this.fg = true;
        if (n === 48) this.bg = true;
        // 38;5;n and 38;2;r;g;b carry their colour in the next parameters.
        if (!sub) i += ps[i + 1] === "5" ? 2 : ps[i + 1] === "2" ? 4 : 0;
      }
    }
  }
}

/** Above this many characters a second (a big `cat`), output goes out as it is until it calms down. */
const FLOOD = 1_000_000;
/** How long a word cut at the end of a write waits for the rest. */
const HOLD_MS = 15;

/** Writes a session's output to its terminal, with highlights while `enabled()`. */
export function highlightedOutput(term: { write(data: string): void }, enabled: () => boolean) {
  let hl = new OutputHighlighter();
  const decoder = new TextDecoder();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let windowStart = 0;
  let windowChars = 0;

  // Highlights must never cost output: if they fail, the text goes out as it came and they start over.
  const safely = (run: () => string, text: string) => {
    const waiting = hl.waiting;
    try {
      return run();
    } catch (e) {
      console.error("output highlighting failed", e);
      hl = new OutputHighlighter();
      return waiting + text;
    }
  };

  const flush = () => {
    clearTimeout(timer);
    timer = undefined;
    const rest = safely(() => hl.flush(enabled()), "");
    if (rest) term.write(rest);
  };

  return {
    write(data: Uint8Array | string) {
      const text = typeof data === "string" ? data : decoder.decode(data, { stream: true });
      const now = performance.now();
      if (now - windowStart > 1000) {
        windowStart = now;
        windowChars = 0;
      }
      windowChars += text.length;
      clearTimeout(timer);
      term.write(safely(() => hl.write(text, enabled() && windowChars < FLOOD), text));
      if (hl.waiting) timer = setTimeout(flush, HOLD_MS);
    },
    /** Before anything else is written to the terminal. */
    flush,
  };
}
