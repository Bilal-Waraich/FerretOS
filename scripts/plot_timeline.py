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
    W, left, right = 960, 150, 20
    plot_w = W - left - right
    x = lambda tick: left + (tick - start) * plot_w / (end - start)
    rows = {"sensor": 20, "logger": 76}
    row_h = 36
    axis_y = rows["logger"] + row_h
    H = axis_y + 92

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
           'font-family="Arial, Helvetica, sans-serif">',
           """<style>
  .bg{fill:#ffffff} .fg{fill:#1a1a1a} .muted{fill:#666666} .grid{stroke:#e6e6e6} .axis{stroke:#808080}
  .work{fill:#bfbfbf} .flush{fill:#808080} .ok{fill:#76b900} .late{fill:#e52020} .rel{stroke:#76b900}
  @media (prefers-color-scheme: dark){
    .bg{fill:#0d1117} .fg{fill:#ededed} .muted{fill:#a0a0a0} .grid{stroke:#21262d} .axis{stroke:#8b949e}
    .work{fill:#444c56} .flush{fill:#768390}
  }
  text{font-size:12px} .name{font-size:13px;font-weight:700} .sub{font-size:11px}
</style>""",
           f'<rect class="bg" width="{W}" height="{H}"/>']

    first = start - start % 10 + (10 if start % 10 else 0)
    for tick in range(first, end + 1, 10):
        out.append(f'<line class="grid" x1="{x(tick):.1f}" y1="{rows["sensor"]}" x2="{x(tick):.1f}" y2="{axis_y}" stroke-width="1"/>')
        out.append(f'<line class="axis" x1="{x(tick):.1f}" y1="{axis_y}" x2="{x(tick):.1f}" y2="{axis_y + 5}" stroke-width="1"/>')
        out.append(f'<text class="muted sub" x="{x(tick):.1f}" y="{axis_y + 19}" text-anchor="middle">{tick}</text>')
    out.append(f'<line class="axis" x1="{left}" y1="{axis_y}" x2="{W - right}" y2="{axis_y}" stroke-width="1"/>')
    out.append(f'<text class="muted sub" x="{left + plot_w / 2:.1f}" y="{axis_y + 38}" text-anchor="middle">time since boot (ms)</text>')

    for key, name, pri in (("sensor", "sensor_reader", "priority 3"), ("logger", "uart_logger", "priority 1")):
        out.append(f'<text class="fg name" x="{left - 14}" y="{rows[key] + 16}" text-anchor="end">{name}</text>')
        out.append(f'<text class="muted sub" x="{left - 14}" y="{rows[key] + 31}" text-anchor="end">{pri}</text>')

    clip = lambda a, b: (max(a, start), min(b, end))
    ly = rows["logger"] + 6
    bh = row_h - 12
    for s in samples:
        a, b = clip(s["t"] - LOGGER_WORK, s["t"])
        if a < b:
            out.append(f'<rect class="work" x="{x(a):.1f}" y="{ly}" width="{x(b) - x(a) - 1:.1f}" height="{bh}"/>')
    for fs, fe in flushes:
        a, b = clip(fs, fe)
        if a < b:
            out.append(f'<rect class="flush" x="{x(a):.1f}" y="{ly}" width="{x(b) - x(a):.1f}" height="{bh}"/>')

    inside = 0
    for s in samples:
        run = s["release"] + s["latency"]
        if not start <= run <= end:
            continue
        cls = "ok" if s["latency"] <= 1 else "late"
        out.append(f'<rect class="{cls}" x="{x(run) - 2:.1f}" y="{rows["sensor"] + 6}" width="4" height="{bh}"/>')
        if any(fs < s["release"] < fe for fs, fe in flushes):
            inside += 1
            out.append(f'<line class="rel" x1="{x(run):.1f}" y1="{rows["sensor"] + row_h - 6}" '
                       f'x2="{x(run):.1f}" y2="{ly}" stroke-width="1.25" stroke-dasharray="3 3"/>')

    ly2 = axis_y + 66
    legend = [("ok", "sensor run"), ("work", "logger line (3 ms)"), ("flush", "logger flush (25 ms)")]
    lx = left
    for cls, label in legend:
        out.append(f'<rect class="{cls}" x="{lx}" y="{ly2 - 10}" width="12" height="12"/>')
        out.append(f'<text class="muted sub" x="{lx + 18}" y="{ly2}">{label}</text>')
        lx += 36 + len(label) * 6
    out.append(f'<line class="rel" x1="{lx + 6}" y1="{ly2 - 11}" x2="{lx + 6}" y2="{ly2 + 3}" stroke-width="1.25" stroke-dasharray="3 3"/>')
    out.append(f'<text class="muted sub" x="{lx + 18}" y="{ly2}">sensor preempts a flush in progress</text>')
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
