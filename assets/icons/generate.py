#!/usr/bin/env python3
"""Генерирует иконки FinanceApp (png, ico, icns) без внешних зависимостей.

Запуск: python3 -I assets/icons/generate.py  (из корня репозитория)
icns собирается через iconutil (только macOS).
"""
import math, os, struct, subprocess, zlib, shutil

OUT = os.path.dirname(os.path.abspath(__file__))
N = 1024


def rrect(x, y, cx, cy, hw, hh, r):
    dx = abs(x - cx) - (hw - r)
    dy = abs(y - cy) - (hh - r)
    return math.hypot(max(dx, 0), max(dy, 0)) + min(max(dx, dy), 0) - r


def cover(d):  # SDF -> покрытие пикселя (антиалиасинг)
    return min(max(0.5 - d, 0.0), 1.0)


def over(dst, src, a):
    return tuple(dst[i] * (1 - a) + src[i] * a for i in range(3))


def pixel(x, y):
    # фон: скруглённый квадрат с вертикальным градиентом
    bg = rrect(x, y, 512, 512, 464, 464, 210)
    a = cover(bg)
    if a == 0:
        return (0, 0, 0, 0)
    t = y / N
    col = (16 + 14 * t, 120 - 40 * t, 110 - 30 * t)
    # столбцы диаграммы
    bars = [(300, 640, 90), (470, 540, 90), (640, 430, 90)]
    for cx, top, hw in bars:
        cy = (top + 800) / 2
        col = over(col, (245, 250, 248), cover(rrect(x, y, cx, cy, hw // 1.4, (800 - top) / 2, 26)))
    # золотая монета
    d = math.hypot(x - 700, y - 330) - 150
    col = over(col, (212, 150, 20), cover(d))
    col = over(col, (250, 200, 70), cover(d + 22))
    col = over(col, (212, 150, 20), cover(abs(math.hypot(x - 700, y - 330) - 80) - 9))
    return (*col, 255 * a)


def render():
    ss = 2
    rows = []
    for j in range(N):
        row = []
        for i in range(N):
            acc = [0.0] * 4
            for sj in range(ss):
                for si in range(ss):
                    p = pixel(i + (si + .5) / ss, j + (sj + .5) / ss)
                    w = p[3] / 255
                    acc[0] += p[0] * w; acc[1] += p[1] * w; acc[2] += p[2] * w; acc[3] += w
            if acc[3] == 0:
                row.append((0, 0, 0, 0))
            else:
                row.append(tuple(int(acc[k] / acc[3]) for k in range(3)) + (int(255 * acc[3] / ss / ss),))
        rows.append(row)
    return rows


def downscale(rows, size):
    f = N // size
    out = []
    for j in range(size):
        row = []
        for i in range(size):
            s = [0, 0, 0, 0]
            for dj in range(f):
                for di in range(f):
                    r, g, b, a = rows[j * f + dj][i * f + di]
                    s[0] += r * a; s[1] += g * a; s[2] += b * a; s[3] += a
            n = f * f
            row.append((0, 0, 0, 0) if s[3] == 0 else
                       (s[0] // s[3], s[1] // s[3], s[2] // s[3], s[3] // n))
        out.append(row)
    return out


def png(rows):
    h, w = len(rows), len(rows[0])
    raw = b"".join(b"\x00" + bytes(c for px in r for c in px) for r in rows)
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def main():
    base = render()
    sizes = [16, 32, 64, 128, 256, 512, 1024]
    pngs = {s: png(base if s == N else downscale(base, s)) for s in sizes}
    open(os.path.join(OUT, "icon.png"), "wb").write(pngs[512])
    # ICO (PNG-кадры, поддерживается Windows Vista+)
    ico_sizes = [16, 32, 64, 128, 256]
    head = struct.pack("<HHH", 0, 1, len(ico_sizes))
    off = 6 + 16 * len(ico_sizes)
    entries, data = b"", b""
    for s in ico_sizes:
        b = pngs[s]
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), off + len(data))
        data += b
    open(os.path.join(OUT, "icon.ico"), "wb").write(head + entries + data)
    # ICNS через iconutil
    if shutil.which("iconutil"):
        iset = os.path.join(OUT, "icon.iconset")
        os.makedirs(iset, exist_ok=True)
        for s in [16, 32, 128, 256, 512]:
            open(os.path.join(iset, f"icon_{s}x{s}.png"), "wb").write(pngs[s])
            open(os.path.join(iset, f"icon_{s}x{s}@2x.png"), "wb").write(pngs[s * 2])
        subprocess.run(["iconutil", "-c", "icns", iset, "-o", os.path.join(OUT, "icon.icns")], check=True)
        shutil.rmtree(iset)


main()
