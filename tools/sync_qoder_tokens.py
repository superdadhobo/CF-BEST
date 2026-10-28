#!/usr/bin/env python3
"""Generate src/theme_tokens.rs from the Qoder Design Kit's tokens.json.

Usage:
    python tools/sync_qoder_tokens.py [path/to/tokens.json]

Default input: %USERPROFILE%/Desktop/QoderDesign/qoder-design-kit/tokens.json

Mapping notes
- Window chrome ("canvas") = fill-tertiary with the theme's window wash painted on top,
  exactly like the kit's `.q-window`. Bee light keeps its 180° gradient (canvas → canvas_end).
- Text-like colors are nudged only when they fall below a contrast floor against every
  surface they sit on (content, cards, chrome): status colors and accent text ≥ 4.5:1,
  tertiary text ≥ 3.4:1, quaternary text ≥ 3:1. Fills, borders and backgrounds are verbatim.
"""
import colorsys
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'src' / 'theme_tokens.rs'
DEFAULT_INPUT = Path(os.environ.get('USERPROFILE', '~')).expanduser() / 'Desktop' / 'QoderDesign' / 'qoder-design-kit' / 'tokens.json'

THEMES = [
    ('FOREST_LIGHT', 'forest-light'),
    ('FOREST_DARK', 'forest-dark'),
    ('MINT_LIGHT', 'mint-light'),
    ('MINT_DARK', 'mint-dark'),
    ('BEE_LIGHT', 'bee-light'),
    ('BEE_DARK', 'bee-dark'),
    ('PARCHMENT_LIGHT', 'light-parchment'),
    ('PARCHMENT_DARK', 'parchment-dark'),
]


# ── color parsing ────────────────────────────────────────────────────────────

def parse_hex(text):
    h = text.strip().lstrip('#')
    if len(h) in (3, 4):
        h = ''.join(c * 2 for c in h)
    if len(h) == 6:
        h += 'ff'
    if len(h) != 8:
        raise ValueError(text)
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4, 6))


def parse_color(text):
    text = text.strip()
    if text == 'white':
        return (1.0, 1.0, 1.0, 1.0)
    if text == 'black':
        return (0.0, 0.0, 0.0, 1.0)
    if text == 'transparent':
        return (0.0, 0.0, 0.0, 0.0)
    m = re.fullmatch(r'color-mix\(in srgb,\s*(.+?)\s+([\d.]+)%,\s*(.+)\)', text)
    if m:
        a, pct, b = parse_color(m.group(1)), float(m.group(2)) / 100, parse_color(m.group(3))
        return mix(a, b, pct)
    m = re.fullmatch(r'linear-gradient\((?:[\d.]+deg,\s*)?(#[0-9a-fA-F]+)(?:\s+0%)?,\s*(#[0-9a-fA-F]+)(?:\s+100%)?\)', text)
    if m:
        return ('gradient', parse_hex(m.group(1)), parse_hex(m.group(2)))
    return parse_hex(text)


def mix(a, b, pa):
    """CSS color-mix in srgb (premultiplied alpha)."""
    pb = 1 - pa
    alpha = a[3] * pa + b[3] * pb
    if alpha == 0:
        return (0.0, 0.0, 0.0, 0.0)
    rgb = tuple((a[i] * a[3] * pa + b[i] * b[3] * pb) / alpha for i in range(3))
    return (*rgb, alpha)


def over(top, base):
    """Composite `top` over an opaque `base`."""
    a = top[3]
    return tuple(top[i] * a + base[i] * (1 - a) for i in range(3)) + (1.0,)


def to_u32(c):
    r, g, b, a = (max(0, min(255, round(v * 255))) for v in c)
    return f'0x{r:02x}{g:02x}{b:02x}{a:02x}'


# ── contrast ─────────────────────────────────────────────────────────────────

def luminance(c):
    def ch(v):
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def ensure_contrast(color, surfaces, target, dark):
    """Mix toward white (dark themes) or black (light themes) until every pair reaches
    `target`. Mixing keeps the hue and softens chroma, unlike shifting HSL lightness."""
    if min(contrast(color, s) for s in surfaces) >= target:
        return color
    toward = (1.0, 1.0, 1.0, 1.0) if dark else (0.0, 0.0, 0.0, 1.0)
    for step in range(1, 101):
        candidate = mix(toward, color, step / 100)
        if min(contrast(candidate, s) for s in surfaces) >= target:
            return candidate
    return color


def oklab_l(c):
    lin = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in c[:3]]
    l = 0.4122214708 * lin[0] + 0.5363325363 * lin[1] + 0.0514459929 * lin[2]
    m = 0.2119034982 * lin[0] + 0.6806995451 * lin[1] + 0.1073969566 * lin[2]
    s = 0.0883024619 * lin[0] + 0.2817188376 * lin[1] + 0.6299787005 * lin[2]
    l, m, s = (v ** (1 / 3) for v in (l, m, s))
    return 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s


def snap_lightness(color, dark):
    """Chart marks: keep the hue, step toward black until OKLCH L sits inside the
    dataviz band for the mode (light 0.43–0.77, dark 0.48–0.67)."""
    high = 0.66 if dark else 0.76
    for step in range(0, 101):
        candidate = mix((0.0, 0.0, 0.0, 1.0), color, step / 100) if step else color
        if oklab_l(candidate) <= high:
            return candidate
    return color


# ── generation ───────────────────────────────────────────────────────────────

FIELDS = [
    # (field, doc)
    ('canvas', 'Window chrome: fill-tertiary + window wash (top stop for gradients).'),
    ('canvas_end', 'Bottom stop of the chrome gradient; equals `canvas` when the wash is flat.'),
    ('content', 'bg-layout: the inset workbench panel.'),
    ('panel', 'bg-container: cards and popovers.'),
    ('elevated', 'bg-elevated: inset groups, table heads, code blocks.'),
    ('input_bg', 'bg-base: inputs and raised controls.'),
    ('fill_secondary', 'fill-secondary: segmented tracks, pressed states.'),
    ('fill_quaternary', 'fill-quaternary: zebra rows.'),
    ('text', 'text'),
    ('muted', 'text-secondary'),
    ('faint', 'text-tertiary (≥ 3.4:1)'),
    ('quaternary', 'text-quaternary (≥ 3:1): sidebar headings, meta.'),
    ('on_accent', 'text-on-primary'),
    ('highlight', 'bg-highlight: tooltips and kbd.'),
    ('on_highlight', 'Text on bg-highlight.'),
    ('border', 'border-tertiary: default 1px border.'),
    ('border_strong', 'border-secondary: hover / emphasis border.'),
    ('border_emphasis', 'border: strongest border, focus.'),
    ('border_subtle', 'border-quaternary: separators inside panels.'),
    ('accent', 'primary'),
    ('accent_hover', 'primary-hover'),
    ('accent_active', 'primary-active'),
    ('accent_text', 'primary-text (≥ 4.5:1)'),
    ('accent_dim', 'primary-bg'),
    ('accent_soft', 'primary-bg-hover'),
    ('focus_ring', 'primary-border: keyboard focus outline'),
    ('green', 'success (≥ 4.5:1)'),
    ('green_dim', 'success-bg'),
    ('red', 'error (≥ 4.5:1)'),
    ('red_dim', 'error-bg'),
    ('red_soft', 'error-bg-hover'),
    ('amber', 'warning (≥ 4.5:1)'),
    ('amber_dim', 'warning-bg'),
    ('blue', 'info (≥ 4.5:1)'),
    ('blue_dim', 'info-bg'),
    ('row_hover', 'sidebar-hover overlay'),
    ('nav_active', 'sidebar-active overlay'),
    ('switch_off_bg', 'switch-off-bg'),
    ('switch_off_border', 'switch-off-border'),
    ('selection', 'primary at 20 % alpha'),
    ('chart_series', 'accent-blue snapped into the dataviz lightness band'),
    ('shadow_2', 'shadow-color-2'),
]


def build(theme_id, theme, wash):
    t = {k: v for k, v in theme['tokens'].items()}
    col = lambda key: parse_color(t[key])
    dark = theme['colorScheme'] == 'dark'

    chrome = col('fill-tertiary')
    wash_value = parse_color(wash) if wash and wash != 'none' else None
    if wash_value and wash_value[0] == 'gradient':
        canvas, canvas_end = over(wash_value[1], chrome), over(wash_value[2], chrome)
    elif wash_value:
        canvas = canvas_end = over(wash_value, chrome)
    else:
        canvas = canvas_end = chrome

    content, panel = col('bg-layout'), col('bg-container')
    # Secondary text also sits on the chrome (sidebar); status / accent text only inside the panel.
    chrome_surfaces = [content, panel, canvas, canvas_end]
    panel_surfaces = [content, panel]
    text_on = lambda key, target: ensure_contrast(col(key), chrome_surfaces, target, dark)
    text_in_panel = lambda key, target: ensure_contrast(col(key), panel_surfaces, target, dark)
    primary = col('primary')

    values = {
        'canvas': canvas,
        'canvas_end': canvas_end,
        'content': content,
        'panel': panel,
        'elevated': col('bg-elevated'),
        'input_bg': col('bg-base'),
        'fill_secondary': col('fill-secondary'),
        'fill_quaternary': col('fill-quaternary'),
        'text': col('text'),
        'muted': col('text-secondary'),
        'faint': text_on('text-tertiary', 3.4),
        'quaternary': text_on('text-quaternary', 3.0),
        'on_accent': col('text-on-primary'),
        'highlight': col('bg-highlight'),
        'on_highlight': col('bg-layout') if not dark else col('bg-base'),
        'border': col('border-tertiary'),
        'border_strong': col('border-secondary'),
        'border_emphasis': col('border'),
        'border_subtle': col('border-quaternary'),
        'accent': primary,
        'accent_hover': col('primary-hover'),
        'accent_active': col('primary-active'),
        'accent_text': text_in_panel('primary-text', 4.5),
        'accent_dim': col('primary-bg'),
        'accent_soft': col('primary-bg-hover'),
        'focus_ring': col('primary-border'),
        'green': text_in_panel('success', 4.5),
        'green_dim': col('success-bg'),
        'red': text_in_panel('error', 4.5),
        'red_dim': col('error-bg'),
        'red_soft': col('error-bg-hover'),
        'amber': text_in_panel('warning', 4.5),
        'amber_dim': col('warning-bg'),
        'blue': text_in_panel('info', 4.5),
        'blue_dim': col('info-bg'),
        'row_hover': col('sidebar-hover'),
        'nav_active': col('sidebar-active'),
        'switch_off_bg': col('switch-off-bg'),
        'switch_off_border': col('switch-off-border'),
        'selection': (*primary[:3], 0.2),
        'chart_series': snap_lightness(col('accent-blue'), dark),
        'shadow_2': col('shadow-color-2'),
    }
    return values, canvas != canvas_end


def main():
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_INPUT
    data = json.loads(src.read_text(encoding='utf-8'))
    washes = data['effects']['windowWash']
    meta = data['meta']

    lines = [
        '// @generated by tools/sync_qoder_tokens.py — do not edit by hand.',
        f'// Source: {meta.get("name", "Qoder Design Kit")} · {meta.get("source", "")}',
        f'// tokens.json generatedAt: {meta.get("generatedAt", "?")}',
        '',
        'use super::Palette;',
        '',
    ]
    for const, theme_id in THEMES:
        theme = data['themes'][theme_id]
        values, gradient = build(theme_id, theme, washes.get(theme_id))
        lines.append(f'/// Qoder `{theme_id}`.')
        lines.append(f'pub(super) const {const}: Palette = Palette {{')
        for field, _doc in FIELDS:
            lines.append(f'    {field}: {to_u32(values[field])},')
        lines.append(f'    canvas_gradient: {"true" if gradient else "false"},')
        lines.append('};')
        lines.append('')
    OUT.write_text('\n'.join(lines), encoding='utf-8', newline='\n')
    print(f'wrote {OUT.relative_to(ROOT)} from {src}')


if __name__ == '__main__':
    main()
