"""Original key art for the demo show in the screenshots: layered landscapes drawn with cairo.

art.py DIR writes key.png and e1.png to e6.png."""
import cairo, math, random, sys

W, H = 1920, 1080

def hexc(h):
    h = h.lstrip('#')
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))

def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))

def ridge(rng, base, amp, rough):
    """Heights across the width: a few octaves of smooth noise."""
    n = 240
    ys = [0.0] * (n + 1)
    for octave in range(1, 6):
        k = 2 ** octave
        pts = [rng.uniform(-1, 1) for _ in range(k + 2)]
        for i in range(n + 1):
            x = i / n * k
            j = int(x); f = x - j
            f = f * f * (3 - 2 * f)
            ys[i] += (pts[j] * (1 - f) + pts[j + 1] * f) * amp * rough ** (octave - 1)
    return [base + y for y in ys]

def scene(path, sky, sun, far, near, seed, sun_pos=(0.68, 0.36), sun_r=90, stars=0, trees=True, water=False):
    rng = random.Random(seed)
    s = cairo.ImageSurface(cairo.FORMAT_RGB24, W, H)
    c = cairo.Context(s)
    g = cairo.LinearGradient(0, 0, 0, H * 0.8)
    for i, col in enumerate(sky):
        g.add_color_stop_rgb(i / (len(sky) - 1), *hexc(col))
    c.set_source(g); c.paint()
    for _ in range(stars):
        x, y = rng.uniform(0, W), rng.uniform(0, H * 0.55)
        a = rng.uniform(0.2, 0.9) * (1 - y / (H * 0.6))
        c.set_source_rgba(1, 1, 1, a)
        c.arc(x, y, rng.choice([0.8, 1.0, 1.4, 2.0]), 0, 7); c.fill()
    sx, sy = sun_pos[0] * W, sun_pos[1] * H
    sc = hexc(sun)
    glow = cairo.RadialGradient(sx, sy, sun_r * 0.5, sx, sy, sun_r * 7)
    glow.add_color_stop_rgba(0, *sc, 0.55); glow.add_color_stop_rgba(0.35, *sc, 0.16); glow.add_color_stop_rgba(1, *sc, 0)
    c.set_source(glow); c.paint()
    c.set_source_rgb(*mix(sc, (1, 1, 1), 0.55)); c.arc(sx, sy, sun_r, 0, 7); c.fill()
    layers = 6
    fc, nc = hexc(far), hexc(near)
    horizon = H * 0.50
    for i in range(layers):
        t = i / (layers - 1)
        base = horizon + t * H * 0.36
        ys = ridge(rng, base, 95 - 45 * t, 0.5)
        col = mix(fc, nc, t ** 0.85)
        c.move_to(0, H)
        for k, y in enumerate(ys):
            c.line_to(k / (len(ys) - 1) * W, y)
        c.line_to(W, H); c.close_path()
        lg = cairo.LinearGradient(0, base - 120, 0, base + 320)
        lg.add_color_stop_rgb(0, *col); lg.add_color_stop_rgb(1, *mix(col, nc, 0.55))
        c.set_source(lg); c.fill()
        # Mist between the ridges.
        if i < layers - 1:
            m = cairo.LinearGradient(0, base + 30, 0, base + 260)
            m.add_color_stop_rgba(0, *mix(fc, (1, 1, 1), 0.35), 0); m.add_color_stop_rgba(1, *mix(fc, (1, 1, 1), 0.35), 0.16 * (1 - t))
            c.set_source(m); c.rectangle(0, base, W, 300); c.fill()
        if trees and i >= layers - 2:
            c.set_source_rgb(*mix(col, nc, 0.6))
            x = rng.uniform(0, 30)
            while x < W:
                y = ys[min(len(ys) - 1, int(x / W * (len(ys) - 1)))]
                h = rng.uniform(40, 110) * (0.7 + 0.6 * t)
                w = h * 0.3
                c.move_to(x, y - h); c.line_to(x + w, y + 12); c.line_to(x - w, y + 12); c.close_path(); c.fill()
                x += rng.uniform(14, 60)
    if water:
        wy = H * 0.86
        wg = cairo.LinearGradient(0, wy, 0, H)
        wg.add_color_stop_rgba(0, *mix(nc, hexc(sky[-1]), 0.18), 1); wg.add_color_stop_rgba(1, *nc, 1)
        c.set_source(wg); c.rectangle(0, wy, W, H - wy); c.fill()
        for k in range(26):
            y = wy + 6 + k * 8
            wdt = sun_r * (0.6 + k * 0.22) * rng.uniform(0.7, 1.2)
            c.set_source_rgba(*mix(sc, (1, 1, 1), 0.4), 0.42 * (1 - k / 26))
            c.rectangle(sx - wdt + rng.uniform(-18, 18), y, wdt * 2, 2.5); c.fill()
    # Vignette.
    v = cairo.RadialGradient(W / 2, H / 2, H * 0.45, W / 2, H / 2, H * 1.15)
    v.add_color_stop_rgba(0, 0, 0, 0, 0); v.add_color_stop_rgba(1, 0, 0, 0, 0.45)
    c.set_source(v); c.paint()
    s.write_to_png(path)

out = sys.argv[1]
P = [
    # name, sky, sun, far, near, kwargs
    ("key",  ["#1b1740", "#4a2a6b", "#c4527a", "#f6a56b", "#ffd9a0"], "#ffd9a0", "#8a5a8e", "#141026", dict(seed=7, sun_pos=(0.70, 0.40), sun_r=80, stars=140, water=True)),
    ("e1",   ["#27406b", "#5d86b8", "#f0c7a0", "#ffe7c4"], "#fff1cf", "#7d95b8", "#16233a", dict(seed=11, sun_pos=(0.30, 0.42), sun_r=70)),
    ("e2",   ["#5a2a3a", "#c8553d", "#f2a65a", "#f8e1a6"], "#fff0b8", "#b0603f", "#2a1418", dict(seed=23, sun_pos=(0.62, 0.38), sun_r=110, trees=False)),
    ("e3",   ["#0c1030", "#1d2a63", "#3f4f9a", "#8a78b8"], "#f3f0ff", "#3d4a86", "#080a1c", dict(seed=31, sun_pos=(0.76, 0.22), sun_r=46, stars=320, water=True)),
    ("e4",   ["#0e3b43", "#1f7a7a", "#7fd1b9", "#e8f6d8"], "#f4ffe6", "#3f8f8a", "#0a2226", dict(seed=47, sun_pos=(0.48, 0.34), sun_r=64, water=True)),
    ("e5",   ["#a9c4e6", "#d6e2f2", "#f7f2ee", "#ffffff"], "#ffffff", "#b9c8dc", "#3c4a66", dict(seed=59, sun_pos=(0.22, 0.26), sun_r=56, trees=False)),
    ("e6",   ["#2b1a4a", "#7b3f8f", "#e56b6f", "#ffb86b"], "#ffe0a8", "#7a4a86", "#1a0f2a", dict(seed=67, sun_pos=(0.40, 0.44), sun_r=96, stars=60)),
]
for name, sky, sun, far, near, kw in P:
    scene(f"{out}/{name}.png", sky, sun, far, near, **kw)
