#!/usr/bin/env python3
"""Plot the sensor-fusion pipeline timeline from a real QEMU run.

Boots the default kernel build under QEMU's deterministic -icount mode (or
reads an existing serial log with --log), then draws an SVG timeline of:

  * sensor_reader runs — one mark per sample at release + latency, coloured
    green when on time, red when late;
  * uart_logger busy time — each logged line is preceded by LOGGER_WORK ticks
    of busy-waiting, and every FLUSH_EVERY lines by a FLUSH_WORK-tick flush.

Every value comes from the kernel's own log lines:

  [LOG] t=<tick> seq=<n> release=<tick> ... latency=<ticks> ...
  [LOG] flush start=<tick> end=<tick>

Usage:
  cargo build --release
  python3 scripts/plot_timeline.py [--log FILE] [--start 30] [--end 130] \
      [--out docs/figures/pipeline_timeline.svg]

The SVG adapts to light and dark themes (prefers-color-scheme).
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
import time

LOGGER_WORK = 3  # must match kernel/src/demo/pipeline.rs
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ELF = os.path.join(ROOT, "target/riscv32imac-unknown-none-elf/release/ferret")


def capture_log(seconds: float) -> str:
    """Boot the kernel in QEMU (-icount, deterministic) and return its output."""
    with tempfile.TemporaryFile() as out:
        qemu = subprocess.Popen(
            ["qemu-system-riscv32", "-machine", "virt", "-nographic", "-bios", "none",
             "-icount", "shift=0", "-kernel", ELF],
            stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        time.sleep(seconds)
        qemu.kill()
        qemu.wait()
        out.seek(0)
        return out.read().decode(errors="replace")


def parse(log: str):
    fields = lambda line: dict(kv.split("=", 1) for kv in line.split() if "=" in kv)
    samples, flushes = [], []
    for line in log.splitlines():
        if line.startswith("[LOG] t="):
            f = fields(line)
            samples.append({k: int(f[k]) for k in ("t", "seq", "release", "latency")})
        elif line.startswith("[LOG] flush start="):
            f = fields(line)
            flushes.append((int(f["start"]), int(f["end"])))
    return samples, flushes


def render(samples, flushes, start, end) -> str:
    W, left, right = 900, 150, 30
    plot_w = W - left - right
    x = lambda tick: left + (tick - start) * plot_w / (end - start)
    rows = {"sensor": 70, "logger": 130}
    row_h = 34
    H = 272

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
           'font-family="-apple-system, BlinkMacSystemFont, \'Segoe UI\', Helvetica, Arial, sans-serif">',
           """<style>
  .bg{fill:#ffffff} .fg{fill:#1f2328} .muted{fill:#59636e} .grid{stroke:#d1d9e0}
  .rowbg{fill:#f6f8fa} .work{fill:#e8a33d} .flush{fill:#bc4c00} .ok{fill:#1a7f37} .late{fill:#cf222e}
  .rel{stroke:#1a7f37}
  @media (prefers-color-scheme: dark){
    .bg{fill:#0d1117} .fg{fill:#e6edf3} .muted{fill:#9198a1} .grid{stroke:#3d444d}
    .rowbg{fill:#151b23} .work{fill:#d29922} .flush{fill:#f0883e} .ok{fill:#3fb950} .late{fill:#f85149}
    .rel{stroke:#3fb950}
  }
  text{font-size:13px}
</style>""",
           f'<rect class="bg" width="{W}" height="{H}" rx="6"/>',
           f'<text class="fg" x="20" y="28" font-size="15" font-weight="600">'
           f'Sensor-fusion pipeline in QEMU — ticks {start}–{end} (1 tick = 1 ms)</text>']

    for name, y in rows.items():
        out.append(f'<rect class="rowbg" x="{left}" y="{y}" width="{plot_w}" height="{row_h}"/>')
    out.append(f'<text class="fg" x="{left - 12}" y="{rows["sensor"] + 15}" text-anchor="end" font-weight="600">sensor_reader</text>')
    out.append(f'<text class="muted" x="{left - 12}" y="{rows["sensor"] + 29}" text-anchor="end" font-size="12">priority 3</text>')
    out.append(f'<text class="fg" x="{left - 12}" y="{rows["logger"] + 15}" text-anchor="end" font-weight="600">uart_logger</text>')
    out.append(f'<text class="muted" x="{left - 12}" y="{rows["logger"] + 29}" text-anchor="end" font-size="12">priority 1</text>')

    for tick in range(start - start % 10 + (10 if start % 10 else 0), end + 1, 10):
        out.append(f'<line class="grid" x1="{x(tick):.1f}" y1="56" x2="{x(tick):.1f}" y2="{rows["logger"] + row_h + 6}" stroke-width="1"/>')
        out.append(f'<text class="muted" x="{x(tick):.1f}" y="{rows["logger"] + row_h + 22}" text-anchor="middle" font-size="12">{tick}</text>')

    clip = lambda a, b: (max(a, start), min(b, end))
    ly = rows["logger"] + 5
    for s in samples:
        a, b = clip(s["t"] - LOGGER_WORK, s["t"])
        if a < b:
            out.append(f'<rect class="work" x="{x(a):.1f}" y="{ly}" width="{x(b) - x(a):.1f}" height="{row_h - 10}" rx="2"/>')
    for fs, fe in flushes:
        a, b = clip(fs, fe)
        if a < b:
            out.append(f'<rect class="flush" x="{x(a):.1f}" y="{ly}" width="{x(b) - x(a):.1f}" height="{row_h - 10}" rx="2"/>')
            out.append(f'<text class="bg" x="{(x(a) + x(b)) / 2:.1f}" y="{ly + 17}" text-anchor="middle" font-size="12" font-weight="600">flush</text>')

    inside = 0
    for s in samples:
        run = s["release"] + s["latency"]
        if not start <= run <= end:
            continue
        cls = "ok" if s["latency"] <= 1 else "late"
        out.append(f'<rect class="{cls}" x="{x(run) - 2:.1f}" y="{rows["sensor"] + 5}" width="5" height="{row_h - 10}" rx="1"/>')
        if any(fs < s["release"] < fe for fs, fe in flushes):
            inside += 1
            out.append(f'<line class="rel" x1="{x(run) + 0.5:.1f}" y1="{rows["sensor"] + row_h}" '
                       f'x2="{x(run) + 0.5:.1f}" y2="{ly}" stroke-width="1.5" stroke-dasharray="3 3"/>')

    ly2 = H - 42
    legend = [("ok", "sensor run (latency ≤ 1 tick)"), ("work", "logger line (3 ticks)"),
              ("flush", "logger flush (25 ticks)")]
    lx = left
    for cls, label in legend:
        out.append(f'<rect class="{cls}" x="{lx}" y="{ly2 - 11}" width="14" height="12" rx="2"/>')
        out.append(f'<text class="muted" x="{lx + 20}" y="{ly2}" font-size="12">{label}</text>')
        lx += 30 + len(label) * 6.6
    out.append(f'<line class="rel" x1="{left + 7}" y1="{ly2 + 10}" x2="{left + 7}" y2="{ly2 + 24}" stroke-width="1.5" stroke-dasharray="3 3"/>')
    out.append(f'<text class="muted" x="{left + 20}" y="{ly2 + 21}" font-size="12">sensor release that falls inside a flush: the sensor preempts the logger and still runs on time</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n", inside


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--log", help="serial log to plot (default: boot QEMU and capture one)")
    ap.add_argument("--seconds", type=float, default=3.0, help="QEMU run time when capturing")
    ap.add_argument("--start", type=int, default=30)
    ap.add_argument("--end", type=int, default=130)
    ap.add_argument("--out", default=os.path.join(ROOT, "docs/figures/pipeline_timeline.svg"))
    args = ap.parse_args()

    log = open(args.log).read() if args.log else capture_log(args.seconds)
    samples, flushes = parse(log)
    if not samples:
        sys.exit("no [LOG] samples found — is this the default (pipeline) build?")
    svg, inside = render(samples, flushes, args.start, args.end)
    with open(args.out, "w") as f:
        f.write(svg)
    late = sum(1 for s in samples if s["latency"] > 1)
    print(f"wrote {args.out}: {len(samples)} samples, {len(flushes)} flushes, "
          f"{inside} sensor runs inside flushes in window, {late} late overall")


if __name__ == "__main__":
    main()
