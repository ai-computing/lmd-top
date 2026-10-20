#!/usr/bin/env python3
"""Sanitize an lmd-top asciicast before publishing it (stdlib only).

The raw recording is replayed through a small terminal model (the subset of ANSI that ratatui and
crossterm emit), names are replaced on the rendered screen rows, and the cast is written back as
row redraws. Working on the screen rather than the byte stream keeps table columns aligned and
also catches names that the TUI only redrew in part.

Replaced:
  * names from an alias file (path in $LMD_DEMO_ALIASES or --aliases; keep it out of git), one
    `real -> alias` per line, `#` comments allowed. Longest names first; a name cut short by the
    TUI (`prefix…`) is replaced by the alias cut to the same width.
  * IPv4 addresses other than loopback -> 192.0.2.x (TEST-NET-1), consistently per address.
  * home directories (`/home/<name>`, `/Users/<name>`) -> `/home/<user>`.

    LMD_DEMO_ALIASES=~/.config/lmd-top/demo-aliases.txt \\
        python3 scripts/sanitize-cast.py raw.cast assets/demo.cast
"""
import argparse
import json
import os
import re
import sys
import unicodedata

IPV4 = re.compile(r"(?<![\d.])((?:25[0-5]|2[0-4]\d|1?\d?\d)(?:\.(?:25[0-5]|2[0-4]\d|1?\d?\d)){3})(?![\d.])")
PRIVATE_IP = re.compile(r"(?<![\d.])(?:10\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])|192\.168)\.\d{1,3}\.\d{1,3}(?![\d.])")
HOME = re.compile(r"/(?:home|Users)/(?!<user>)[A-Za-z0-9._-]+")
CSI = re.compile(r"\x1b\[([?>=]?)([0-9;:]*)([ -/]*)([@-~])")
INCOMPLETE = re.compile(r"\x1b(?:\[[?>=]?[0-9;:]*[ -/]*|\][^\x07\x1b]*|[()])?$")
FILL = set(" ▀▄─━═")  # blank, half blocks, box lines
BORDERS = set("│┃║|╭╮╰╯┌┐└┘├┤")
OTHER_ESC = re.compile(r"\x1b(?:\][^\x07\x1b]*(?:\x07|\x1b\\)|[()][0-9A-Za-z]|[78=>cDEHMNOZ])")

DEFAULT = (None, None, frozenset())  # fg, bg, attrs


def load_aliases(path):
    pairs = []
    with open(os.path.expanduser(path), encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if "->" not in line:
                continue
            real, alias = (s.strip() for s in line.split("->", 1))
            alias = alias.split()[0] if alias else ""
            if real and alias:
                pairs.append((real, alias))
    pairs.sort(key=lambda p: -len(p[0]))
    return pairs


def width(ch):
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.cells = [[(" ", DEFAULT) for _ in range(cols)] for _ in range(rows)]
        self.x = self.y = 0
        self.style = DEFAULT
        self.partial = ""  # escape sequence cut off at the end of the previous chunk

    def sgr(self, params):
        fg, bg, attrs = self.style
        attrs = set(attrs)
        p = [int(v) if v else 0 for v in re.split("[;:]", params)] if params else [0]
        i = 0
        while i < len(p):
            v = p[i]
            if v == 0:
                fg, bg, attrs = None, None, set()
            elif v in (38, 48):
                if i + 1 < len(p) and p[i + 1] == 2:
                    col = "2;%d;%d;%d" % tuple((p[i + 2:i + 5] + [0, 0, 0])[:3])
                    i += 4
                else:
                    col = "5;%d" % (p[i + 2] if i + 2 < len(p) else 0)
                    i += 2
                if v == 38:
                    fg = "38;" + col
                else:
                    bg = "48;" + col
            elif v == 39:
                fg = None
            elif v == 49:
                bg = None
            elif 30 <= v <= 37 or 90 <= v <= 97:
                fg = str(v)
            elif 40 <= v <= 47 or 100 <= v <= 107:
                bg = str(v)
            elif v in (1, 2, 3, 4, 5, 7, 8, 9):
                attrs.add(v)
            elif v == 22:
                attrs -= {1, 2}
            elif 23 <= v <= 29:
                attrs.discard(v - 20)
            i += 1
        self.style = (fg, bg, frozenset(attrs))

    def put(self, ch):
        w = width(ch)
        if w == 0:
            return
        if self.x >= self.cols:
            self.x = 0
            self.y = min(self.y + 1, self.rows - 1)
        self.cells[self.y][self.x] = (ch, self.style)
        if w == 2 and self.x + 1 < self.cols:
            self.cells[self.y][self.x + 1] = ("", self.style)
        self.x += w

    def erase(self, row, x0, x1):
        for x in range(x0, x1):
            self.cells[row][x] = (" ", (None, self.style[1], frozenset()))

    def feed(self, text):
        text, self.partial = self.partial + text, ""
        i = 0
        while i < len(text):
            ch = text[i]
            if ch == "\x1b":
                m = CSI.match(text, i)
                if m:
                    self.csi(m.group(1), m.group(2), m.group(4))
                    i = m.end()
                    continue
                m = OTHER_ESC.match(text, i)
                if m:
                    i = m.end()
                elif INCOMPLETE.match(text, i):
                    self.partial = text[i:]
                    return
                else:
                    i += 1
                continue
            if ch == "\r":
                self.x = 0
            elif ch == "\n":
                self.y = min(self.y + 1, self.rows - 1)
            elif ch == "\b":
                self.x = max(self.x - 1, 0)
            elif ch >= " ":
                self.put(ch)
            i += 1

    def csi(self, priv, params, final):
        if priv:
            return  # modes (alt screen, cursor visibility, mouse): no effect on cell content
        nums = [int(v) if v else 0 for v in params.split(";")] if params else []
        n = (nums[0] or 1) if nums else 1
        if final == "m":
            self.sgr(params)
        elif final in "Hf":
            r = nums[0] if nums else 1
            c = nums[1] if len(nums) > 1 else 1
            self.y = min(max(r, 1), self.rows) - 1
            self.x = min(max(c, 1), self.cols) - 1
        elif final == "A":
            self.y = max(self.y - n, 0)
        elif final == "B":
            self.y = min(self.y + n, self.rows - 1)
        elif final == "C":
            self.x = min(self.x + n, self.cols - 1)
        elif final == "D":
            self.x = max(self.x - n, 0)
        elif final == "G":
            self.x = min(n, self.cols) - 1
        elif final == "d":
            self.y = min(n, self.rows) - 1
        elif final == "J":
            mode = nums[0] if nums else 0
            if mode in (2, 3):
                for r in range(self.rows):
                    self.erase(r, 0, self.cols)
            elif mode == 0:
                self.erase(self.y, self.x, self.cols)
                for r in range(self.y + 1, self.rows):
                    self.erase(r, 0, self.cols)
            elif mode == 1:
                for r in range(self.y):
                    self.erase(r, 0, self.cols)
                self.erase(self.y, 0, self.x + 1)
        elif final == "K":
            mode = nums[0] if nums else 0
            a, b = {0: (self.x, self.cols), 1: (0, self.x + 1)}.get(mode, (0, self.cols))
            self.erase(self.y, a, b)
        elif final == "X":
            self.erase(self.y, self.x, min(self.x + n, self.cols))


class Scrubber:
    def __init__(self, aliases):
        self.names = {}  # visible text -> replacement of the same meaning
        for real, alias in aliases:
            self.names.setdefault(real, alias)
            # Cut-short forms the TUI renders as `prefix…` (at least 6 visible characters).
            for k in range(len(real) - 1, 5, -1):
                self.names.setdefault(real[:k] + "…", alias if len(alias) <= k else alias[:k] + "…")
        self.full = [(r, a) for r, a in aliases if len(r) >= 6]
        alt = "|".join(re.escape(n) for n in sorted(self.names, key=len, reverse=True))
        self.rules = [(re.compile(alt), self.names.get)] if alt else []
        self.rules += [(IPV4, self.ip_alias), (HOME, lambda _: "/home/<user>")]
        self.ips = {}
        self.count = 0

    def ip_alias(self, ip):
        if ip.startswith("127.") or ip == "0.0.0.0":
            return ip
        if ip not in self.ips:
            self.ips[ip] = "192.0.2.%d" % (len(self.ips) + 10)
        return self.ips[ip]

    def screen(self, rows):
        """Scrub a whole screen (list of rows of (char, style)); returns new rows."""
        rows = [list(r) for r in rows]
        self.wrapped(rows)
        return [self.row(r) for r in rows]

    def wrapped(self, rows):
        """Names a panel wrapped onto the next line: `…host-0│` then `│1/…` (character wrap)."""
        texts = [self.text(r) for r in rows]
        for y in range(len(rows) - 1):
            for real, alias in self.full:
                for k in range(2, len(real) - 1):
                    head, tail = real[:k], real[k:]
                    for m in re.finditer(re.escape(head) + r"(?=[│┃|]|$)", texts[y]):
                        for n in re.finditer(r"(?:(?<=[│┃|])|^)" + re.escape(tail), texts[y + 1]):
                            rows[y] = self.splice(rows[y], m.start(), m.end(), alias[:k])
                            rows[y + 1] = self.splice(rows[y + 1], n.start(), n.end(), alias[k:])
                            texts[y], texts[y + 1] = self.text(rows[y]), self.text(rows[y + 1])
                            self.count += 1

    @staticmethod
    def text(cells):
        return "".join(c if c else "\0" for c, _ in cells)

    def row(self, cells):
        """Replace names in one screen row, keeping the row width."""
        for rx, rep in self.rules:
            text = self.text(cells)
            pos = 0
            while True:
                m = rx.search(text, pos)
                if not m:
                    break
                new = rep(m.group(0))
                if new == m.group(0):
                    pos = m.end()
                    continue
                self.count += 1
                cells = self.splice(cells, m.start(), m.end(), new)
                text = self.text(cells)
                pos = m.start() + len(new)
        return cells

    @staticmethod
    def splice(cells, a, b, new):
        """Put `new` over cells[a:b] without moving anything outside the panel segment.

        The segment runs from `b` to the next box-drawing border. A longer replacement takes
        its room from the first run of blanks in the segment (keeping one blank as separator),
        then from the segment end; a shorter one gives the room back at the same place.
        """
        style = cells[a][1]
        end = b
        while end < len(cells) and cells[end][0] not in BORDERS:
            end += 1
        seg = cells[b:end]
        delta = len(new) - (b - a)
        # Run that absorbs the size change: blanks or a drawn line (`▀▀▀`, `───`) directly after
        # the name, else the first run of blanks.
        if seg and seg[0][0] in FILL:
            i = 0
        else:
            i = next((k for k, c in enumerate(seg) if c[0] == " "), len(seg))
        fill = seg[i][0] if i < len(seg) else " "
        j = i
        while j < len(seg) and seg[j][0] == fill:
            j += 1
        if delta < 0:
            pad = (fill, seg[i][1] if i < len(seg) else style)
            seg = seg[:i] + [pad] * -delta + seg[i:]
        elif delta > 0:
            # A blank run before more text stays one blank wide.
            keep = 1 if j < len(seg) and fill == " " else 0
            take = min(delta, max(j - i - keep, 0))
            seg = seg[:i] + seg[i + take:]
            short = delta - take
            if short >= len(seg):
                new = new[:len(new) - (short - len(seg))]
                seg = []
            elif short:
                seg = seg[:len(seg) - short]
                seg[-1] = ("…", seg[-1][1])
        return cells[:a] + [(ch, style) for ch in new] + seg + cells[end:]


def emit_row(y, cells):
    parts = ["\x1b[%d;1H" % (y + 1)]
    cur = None
    for ch, st in cells:
        if ch == "":
            continue
        if st != cur:
            fg, bg, attrs = st
            codes = ["0"] + [str(a) for a in sorted(attrs)]
            if fg:
                codes.append(fg)
            if bg:
                codes.append(bg)
            parts.append("\x1b[%sm" % ";".join(codes))
            cur = st
        parts.append(ch)
    parts.append("\x1b[0m")
    return "".join(parts)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("raw")
    ap.add_argument("out")
    ap.add_argument("--aliases", default=os.environ.get("LMD_DEMO_ALIASES"))
    ap.add_argument("--max-gap", type=float, default=1.5, help="cap idle time between frames (s)")
    ap.add_argument("--fps", type=float, default=15.0, help="merge output into frames at this rate")
    ap.add_argument("--start", type=float, default=0.0,
                    help="drop frames before this time (s), e.g. the first data load")
    a = ap.parse_args()
    if not a.aliases:
        ap.error("alias file required: set LMD_DEMO_ALIASES or pass --aliases")
    scrub = Scrubber(load_aliases(a.aliases))

    with open(a.raw, encoding="utf-8") as f:
        header = json.loads(f.readline())
        events = [json.loads(l) for l in f if l.strip()]
    events = [e for e in events if e[1] == "o"]
    cols, rows = header["width"], header["height"]
    scr = Screen(cols, rows)
    shown = [None] * rows
    out_events = []
    clock, last_raw = 0.0, a.start
    step = 1.0 / a.fps
    i = 0
    while i < len(events):
        # One frame = everything printed within `step` seconds of the first event.
        t0 = events[i][0]
        while i < len(events) and events[i][0] - t0 < step:
            scr.feed(events[i][2])
            i += 1
        if t0 < a.start:
            continue
        rows_out = []
        for y, cells in enumerate(scrub.screen(scr.cells)):
            if cells != shown[y]:
                shown[y] = cells
                rows_out.append(emit_row(y, cells))
        if not rows_out:
            continue
        clock += min(t0 - last_raw, a.max_gap)
        last_raw = t0
        out_events.append([round(clock, 3), "o", "".join(rows_out)])

    head = {"version": 2, "width": cols, "height": rows, "env": {"TERM": "xterm-256color"},
            "title": header.get("title", "lmd-top")}
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(head) + "\n")
        f.write(json.dumps([0.0, "o", "\x1b[?25l\x1b[2J"]) + "\n")
        for e in out_events:
            f.write(json.dumps(e, ensure_ascii=False) + "\n")
    print("frames: %d, duration: %.1fs, replacements: %d, distinct IPv4 mapped: %d"
          % (len(out_events), clock, scrub.count, len(scrub.ips)), file=sys.stderr)

    # Leak check on the written file (counts only, so the names stay out of logs).
    with open(a.out, encoding="utf-8") as f:
        text = "".join(json.loads(l)[2] if l.startswith("[") else "" for l in f)
    left = sum(text.count(real) for real, _ in load_aliases(a.aliases) if len(real) >= 4)
    left += len(PRIVATE_IP.findall(text)) + len(HOME.findall(text))
    if left:
        sys.exit("sanitize-cast: %d sensitive matches left in %s; do not publish it" % (left, a.out))


if __name__ == "__main__":
    main()
